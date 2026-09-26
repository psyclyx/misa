//! Measured, protocol-free standard controls. Callers own values and focus;
//! placed controls share their paint, clip and hit geometry.
use crate::{Op, Rect, TextMetrics};
use misa_style::Style;
use std::sync::Arc;

fn clip(bounds: Rect, ops: Vec<Op>) -> Vec<Op> {
    vec![Op::ClipRect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width.max(0.0),
        height: bounds.height.max(0.0),
        ops: Arc::new(ops),
    }]
}

fn text(
    bounds: Rect,
    label: String,
    size: f32,
    metrics: &dyn TextMetrics,
    color: Style,
) -> (f32, Op) {
    let width = metrics.measure(&label, size);
    let height = metrics.line_metrics(size).line_height;
    assert!(height > 0.0, "widget requires positive line spacing");
    (
        width,
        Op::Text {
            x: bounds.x,
            y: bounds.y + ((bounds.height - height) / 2.0).max(0.0),
            size,
            style: color,
            text: label,
        },
    )
}

pub struct PlacedWidget<Id> {
    pub id: Id,
    pub bounds: Rect,
    pub label_width: f32,
    pub ops: Vec<Op>,
}
impl<Id> PlacedWidget<Id> {
    pub fn hit(&self, x: f32, y: f32) -> bool {
        self.bounds.contains(x, y)
    }
}

pub struct Label<Id> {
    pub id: Id,
    pub bounds: Rect,
    pub text: String,
    pub font_size: f32,
    pub foreground: Style,
}
impl<Id> Label<Id> {
    pub fn place(self, metrics: &dyn TextMetrics) -> PlacedWidget<Id> {
        let (label_width, op) = text(
            self.bounds,
            self.text,
            self.font_size,
            metrics,
            self.foreground,
        );
        PlacedWidget {
            id: self.id,
            bounds: self.bounds,
            label_width,
            ops: clip(self.bounds, vec![op]),
        }
    }
}

/// Activation returns the caller's new value; the widget does not retain state.
pub struct Checkbox<Id> {
    pub id: Id,
    pub bounds: Rect,
    pub label: String,
    pub checked: bool,
    pub focused: bool,
    pub font_size: f32,
    pub background: Style,
    pub foreground: Style,
    pub accent: Style,
}
impl<Id> Checkbox<Id> {
    pub fn place(self, metrics: &dyn TextMetrics) -> PlacedCheckbox<Id> {
        let b = self.bounds;
        let side = (b.height - 8.0).max(0.0).min(18.0);
        let box_bounds = Rect {
            x: b.x + 4.0,
            y: b.y + (b.height - side) / 2.0,
            width: side,
            height: side,
        };
        let label_bounds = Rect {
            x: box_bounds.x + side + 8.0,
            y: b.y,
            width: (b.width - side - 12.0).max(0.0),
            height: b.height,
        };
        let (label_width, label) = text(
            label_bounds,
            self.label,
            self.font_size,
            metrics,
            self.foreground,
        );
        let mut ops = vec![
            Op::Rect {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
                style: if self.focused {
                    self.accent
                } else {
                    self.background
                },
            },
            Op::Rect {
                x: box_bounds.x,
                y: box_bounds.y,
                width: side,
                height: side,
                style: self.foreground,
            },
        ];
        if side > 4.0 {
            ops.push(Op::Rect {
                x: box_bounds.x + 2.0,
                y: box_bounds.y + 2.0,
                width: side - 4.0,
                height: side - 4.0,
                style: if self.checked {
                    self.accent
                } else {
                    self.background
                },
            });
        }
        ops.push(label);
        PlacedCheckbox {
            widget: PlacedWidget {
                id: self.id,
                bounds: b,
                label_width,
                ops: clip(b, ops),
            },
            checked: self.checked,
        }
    }
}
pub struct PlacedCheckbox<Id> {
    pub widget: PlacedWidget<Id>,
    pub checked: bool,
}
impl<Id: Clone> PlacedCheckbox<Id> {
    pub fn click(&self, x: f32, y: f32) -> Option<(Id, bool)> {
        self.widget.hit(x, y).then(|| self.space())
    }
    pub fn space(&self) -> (Id, bool) {
        (self.widget.id.clone(), !self.checked)
    }
}

/// A radio option yields its group value on activation. The caller owns the group.
pub struct RadioButton<Id, Value> {
    pub id: Id,
    pub value: Value,
    pub selected: bool,
    pub bounds: Rect,
    pub label: String,
    pub focused: bool,
    pub font_size: f32,
    pub background: Style,
    pub foreground: Style,
    pub accent: Style,
}
pub struct PlacedRadioButton<Id, Value> {
    pub widget: PlacedWidget<Id>,
    pub value: Value,
}
impl<Id, Value> RadioButton<Id, Value> {
    pub fn place(self, metrics: &dyn TextMetrics) -> PlacedRadioButton<Id, Value> {
        let b = self.bounds;
        let side = (b.height - 8.0).max(0.0).min(18.0);
        let x = b.x + 4.0;
        let y = b.y + (b.height - side) / 2.0;
        let label_bounds = Rect {
            x: x + side + 8.0,
            y: b.y,
            width: (b.width - side - 12.0).max(0.0),
            height: b.height,
        };
        let (label_width, label) = text(
            label_bounds,
            self.label,
            self.font_size,
            metrics,
            self.foreground,
        );
        let mut ops = vec![Op::Rect {
            x: b.x,
            y: b.y,
            width: b.width,
            height: b.height,
            style: if self.focused {
                self.accent
            } else {
                self.background
            },
        }];
        // Pixel diamond ring and center dot distinguish a radio choice from a
        // checkbox; all paint still uses clipped Scene operations.
        let step = side / 7.0;
        for row in 0..7 {
            let inset = (3_i32 - row as i32).unsigned_abs() as f32;
            let left = x + inset * step;
            let right = x + side - (inset + 1.0) * step;
            ops.push(Op::Rect {
                x: left,
                y: y + row as f32 * step,
                width: step,
                height: step,
                style: self.foreground,
            });
            ops.push(Op::Rect {
                x: right,
                y: y + row as f32 * step,
                width: step,
                height: step,
                style: self.foreground,
            });
        }
        if self.selected {
            ops.push(Op::Rect {
                x: x + 2.0 * step,
                y: y + 2.0 * step,
                width: 2.0 * step,
                height: 2.0 * step,
                style: self.accent,
            });
        }
        ops.push(label);
        PlacedRadioButton {
            widget: PlacedWidget {
                id: self.id,
                bounds: b,
                label_width,
                ops: clip(b, ops),
            },
            value: self.value,
        }
    }
}
impl<Id: Clone, Value: Clone> PlacedRadioButton<Id, Value> {
    pub fn click(&self, x: f32, y: f32) -> Option<(Id, Value)> {
        self.widget.hit(x, y).then(|| self.space())
    }
    pub fn space(&self) -> (Id, Value) {
        (self.widget.id.clone(), self.value.clone())
    }
}

pub struct ProgressBar<Id> {
    pub id: Id,
    pub bounds: Rect,
    /// Clamped to [0, 1]; NaN is treated as zero.
    pub fraction: f32,
    pub background: Style,
    pub foreground: Style,
}
impl<Id> ProgressBar<Id> {
    pub fn place(self) -> PlacedWidget<Id> {
        let b = self.bounds;
        let fraction = if self.fraction.is_nan() {
            0.0
        } else {
            self.fraction.clamp(0.0, 1.0)
        };
        PlacedWidget {
            id: self.id,
            bounds: b,
            label_width: 0.0,
            ops: clip(
                b,
                vec![
                    Op::Rect {
                        x: b.x,
                        y: b.y,
                        width: b.width,
                        height: b.height,
                        style: self.background,
                    },
                    Op::Rect {
                        x: b.x,
                        y: b.y,
                        width: b.width.max(0.0) * fraction,
                        height: b.height,
                        style: self.foreground,
                    },
                ],
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKey {
    Up,
    Down,
    Home,
    End,
    Enter,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ListBoxState {
    pub selected: Option<usize>,
    pub highlighted: Option<usize>,
    /// Pixel offset, reconciled whenever the list is placed.
    pub scroll: f32,
}

/// The caller supplies the number of rows and a label factory; only rows in
/// the viewport are requested during placement.
pub struct ListBox<Id, Label> {
    pub id: Id,
    pub bounds: Rect,
    pub count: usize,
    pub label: Label,
    pub font_size: f32,
    pub row_height: f32,
    pub focused: bool,
    pub background: Style,
    pub foreground: Style,
    pub highlight: Style,
}
pub struct PlacedListBox<Id> {
    pub widget: PlacedWidget<Id>,
    pub row_height: f32,
    pub count: usize,
    pub scroll: f32,
}
impl<Id, Label: Fn(usize) -> String> ListBox<Id, Label> {
    pub fn place(self, metrics: &dyn TextMetrics, state: &mut ListBoxState) -> PlacedListBox<Id> {
        assert!(self.row_height > 0.0, "list requires positive row height");
        let b = self.bounds;
        let count = self.count;
        if state.selected.is_some_and(|i| i >= count) {
            state.selected = None;
        }
        if state.highlighted.is_some_and(|i| i >= count) {
            state.highlighted = None;
        }
        state.scroll = state
            .scroll
            .max(0.0)
            .min((count as f32 * self.row_height - b.height.max(0.0)).max(0.0));
        let mut ops = vec![Op::Rect {
            x: b.x,
            y: b.y,
            width: b.width,
            height: b.height,
            style: self.background,
        }];
        let mut label_width: f32 = 0.0;
        // The viewport intersects [floor(scroll / row_height),
        // ceil((scroll + height) / row_height)); never traverse the model.
        let start = ((state.scroll / self.row_height).floor() as usize).min(count);
        let end =
            (((state.scroll + b.height.max(0.0)) / self.row_height).ceil() as usize).min(count);
        for index in start..end {
            let y = b.y + index as f32 * self.row_height - state.scroll;
            if y + self.row_height <= b.y || y >= b.y + b.height {
                continue;
            }
            let row = Rect {
                x: b.x,
                y,
                width: b.width,
                height: self.row_height,
            };
            let mut row_ops = Vec::new();
            if (state.highlighted == Some(index) && self.focused) || state.selected == Some(index) {
                row_ops.push(Op::Rect {
                    x: row.x,
                    y,
                    width: row.width,
                    height: row.height,
                    style: self.highlight,
                });
            }
            let label_rect = Rect {
                x: b.x + 4.0,
                y,
                width: (b.width - 8.0).max(0.0),
                height: self.row_height,
            };
            let (width, label) = text(
                label_rect,
                (self.label)(index),
                self.font_size,
                metrics,
                self.foreground,
            );
            label_width = label_width.max(width);
            row_ops.push(label);
            ops.extend(clip(row, row_ops));
        }
        PlacedListBox {
            widget: PlacedWidget {
                id: self.id,
                bounds: b,
                label_width,
                ops: clip(b, ops),
            },
            row_height: self.row_height,
            count,
            scroll: state.scroll,
        }
    }
}
impl<Id: Clone> PlacedListBox<Id> {
    /// Commit a visible row; clicks outside the clip cannot select hidden rows.
    pub fn click(&self, state: &mut ListBoxState, x: f32, y: f32) -> Option<(Id, usize)> {
        if !self.widget.hit(x, y) {
            return None;
        }
        let index = ((y - self.widget.bounds.y + self.scroll) / self.row_height).floor() as usize;
        if index >= self.count {
            return None;
        }
        state.highlighted = Some(index);
        state.selected = Some(index);
        Some((self.widget.id.clone(), index))
    }
    pub fn wheel(&self, state: &mut ListBoxState, delta: f32) {
        if delta.is_finite() {
            state.scroll = (self.scroll + delta).clamp(0.0, self.max_scroll());
        }
    }
    fn max_scroll(&self) -> f32 {
        (self.count as f32 * self.row_height - self.widget.bounds.height.max(0.0)).max(0.0)
    }
    pub fn key(&self, state: &mut ListBoxState, key: ListKey) -> Option<(Id, usize)> {
        if self.count == 0 {
            return None;
        }
        let current = state.highlighted.or(state.selected);
        let index = match key {
            ListKey::Up => current.unwrap_or(0).saturating_sub(1),
            ListKey::Down => current.map_or(0, |n| (n + 1).min(self.count - 1)),
            ListKey::Home => 0,
            ListKey::End => self.count - 1,
            ListKey::Enter => {
                let index = current?;
                state.selected = Some(index);
                return Some((self.widget.id.clone(), index));
            }
        };
        state.highlighted = Some(index);
        let top = index as f32 * self.row_height;
        state.scroll = self
            .scroll
            .min(top)
            .max(top + self.row_height - self.widget.bounds.height.max(0.0))
            .clamp(0.0, self.max_scroll());
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure(&self, text: &str, size: f32) -> f32 {
            text.chars().count() as f32 * size
        }
        fn advances(&self, text: &str, size: f32) -> Vec<f32> {
            (0..=text.chars().count())
                .map(|i| i as f32 * size)
                .collect()
        }
        fn line_metrics(&self, size: f32) -> crate::LineMetrics {
            crate::LineMetrics {
                ascent: -size,
                descent: 0.0,
                leading: 0.0,
                line_height: size,
            }
        }
    }
    fn bounds(width: f32) -> Rect {
        Rect {
            x: 10.0,
            y: 20.0,
            width,
            height: 26.0,
        }
    }
    #[test]
    fn checkbox_radio_label_progress_are_measured_clipped_and_typed() {
        for width in [16.0, 120.0] {
            let c = Checkbox {
                id: "check",
                bounds: bounds(width),
                label: "wide text".into(),
                checked: false,
                focused: true,
                font_size: 10.0,
                background: Style::default(),
                foreground: Style::default(),
                accent: Style::default(),
            }
            .place(&Metrics);
            assert_eq!(c.widget.label_width, 90.0);
            assert_eq!(c.click(10.0, 25.0), Some(("check", true)));
            assert_eq!(c.click(10.0 + width, 25.0), None);
            assert!(matches!(&c.widget.ops[0], Op::ClipRect { width: w, .. } if *w == width));
            let r = RadioButton {
                id: 2u8,
                value: "group-value",
                selected: true,
                bounds: bounds(width),
                label: "radio".into(),
                focused: false,
                font_size: 10.0,
                background: Style::default(),
                foreground: Style::default(),
                accent: Style::default(),
            }
            .place(&Metrics);
            assert_eq!(r.space(), (2, "group-value"));
            assert_eq!(r.click(10.0 + width, 25.0), None);
            let label = Label {
                id: 3,
                bounds: bounds(width),
                text: "abc".into(),
                font_size: 10.0,
                foreground: Style::default(),
            }
            .place(&Metrics);
            assert_eq!(label.label_width, 30.0);
            assert!(matches!(label.ops[0], Op::ClipRect { .. }));
            let bar = ProgressBar {
                id: 4,
                bounds: bounds(width),
                fraction: 1.5,
                background: Style::default(),
                foreground: Style::default(),
            }
            .place();
            assert!(
                matches!(&bar.ops[0], Op::ClipRect { ops, .. } if matches!(ops[1], Op::Rect { width: w, .. } if w == width))
            );
        }
    }
    #[test]
    fn large_list_requests_only_visible_labels_and_tracks_indices_after_append() {
        use std::cell::RefCell;
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct CountingMetrics(AtomicUsize);
        impl TextMetrics for CountingMetrics {
            fn measure(&self, text: &str, size: f32) -> f32 {
                self.0.fetch_add(1, Ordering::Relaxed);
                Metrics.measure(text, size)
            }
            fn advances(&self, text: &str, size: f32) -> Vec<f32> {
                Metrics.advances(text, size)
            }
            fn line_metrics(&self, size: f32) -> crate::LineMetrics {
                Metrics.line_metrics(size)
            }
        }
        let metrics = CountingMetrics(AtomicUsize::new(0));
        let calls = RefCell::new(Vec::new());
        let mut state = ListBoxState::default();
        let list = |count| ListBox {
            id: 7,
            bounds: Rect {
                x: 10.0,
                y: 20.0,
                width: 80.0,
                height: 45.0,
            },
            count,
            label: |index| {
                calls.borrow_mut().push(index);
                format!("item {index}")
            },
            font_size: 10.0,
            row_height: 20.0,
            focused: true,
            background: Style::default(),
            foreground: Style::default(),
            highlight: Style::default(),
        };
        let head = list(100_000).place(&metrics, &mut state);
        assert_eq!(*calls.borrow(), [0, 1, 2]);
        assert!(calls.borrow().len() <= 3 + 2);
        assert_eq!(metrics.0.swap(0, Ordering::Relaxed), calls.borrow().len());
        assert_eq!(head.click(&mut state, 11.0, 64.0), Some((7, 2)));
        head.wheel(&mut state, 10_000_000.0);
        calls.borrow_mut().clear();
        let tail = list(100_000).place(&metrics, &mut state);
        assert_eq!(*calls.borrow(), [99_997, 99_998, 99_999]);
        assert!(calls.borrow().len() <= 3 + 2);
        assert_eq!(metrics.0.load(Ordering::Relaxed), calls.borrow().len());
        assert_eq!(tail.click(&mut state, 11.0, 64.0), Some((7, 99_999)));
        assert_eq!(tail.key(&mut state, ListKey::Enter), Some((7, 99_999)));
        tail.key(&mut state, ListKey::Home);
        assert_eq!(state.scroll, 0.0);
        tail.key(&mut state, ListKey::End);
        assert_eq!(state.highlighted, Some(99_999));
        assert_eq!(state.scroll, 1_999_955.0);

        // Appending keeps the old selection and extends the scroll range.
        calls.borrow_mut().clear();
        let appended = list(100_001).place(&metrics, &mut state);
        assert_eq!(state.selected, Some(99_999));
        assert!(calls.borrow().len() <= 3 + 2);
        appended.wheel(&mut state, 10_000.0);
        calls.borrow_mut().clear();
        let appended = list(100_001).place(&metrics, &mut state);
        assert_eq!(*calls.borrow(), [99_998, 99_999, 100_000]);
        assert_eq!(appended.click(&mut state, 11.0, 64.0), Some((7, 100_000)));
        assert_eq!(state.scroll, 1_999_975.0);
        calls.borrow_mut().clear();
        let smaller = list(2).place(&metrics, &mut state);
        assert_eq!(state.scroll, 0.0);
        assert_eq!(state.selected, None);
        assert_eq!(state.highlighted, None);
        assert_eq!(*calls.borrow(), [0, 1]);
        assert_eq!(smaller.click(&mut state, 11.0, 64.0), None);
    }

    #[test]
    fn list_keyboard_pointer_scroll_and_resize_share_visible_clip() {
        let mut state = ListBoxState::default();
        let list = |height| ListBox {
            id: 7,
            bounds: Rect {
                x: 10.0,
                y: 20.0,
                width: 35.0,
                height,
            },
            count: 10,
            label: |i| format!("item {i}"),
            font_size: 10.0,
            row_height: 20.0,
            focused: true,
            background: Style::default(),
            foreground: Style::default(),
            highlight: Style::default(),
        };
        let p = list(40.0).place(&Metrics, &mut state);
        assert!(matches!(&p.widget.ops[0], Op::ClipRect { ops, .. }
            if matches!(&ops[1], Op::ClipRect { y: 20.0, height: 20.0, ops, .. }
                if matches!(&ops[0], Op::Text { text, .. } if text == "item 0"))));
        assert_eq!(p.click(&mut state, 45.0, 25.0), None);
        assert_eq!(p.click(&mut state, 11.0, 25.0), Some((7, 0)));
        assert_eq!(p.key(&mut state, ListKey::End), None);
        assert_eq!(state.highlighted, Some(9));
        assert_eq!(state.scroll, 160.0);
        let p = list(40.0).place(&Metrics, &mut state);
        assert_eq!(p.click(&mut state, 11.0, 59.0), Some((7, 9)));
        assert_eq!(p.click(&mut state, 11.0, 60.0), None);
        p.key(&mut state, ListKey::Up);
        assert_eq!(p.key(&mut state, ListKey::Enter), Some((7, 8)));
        p.key(&mut state, ListKey::Home);
        assert_eq!(state.scroll, 0.0);
        p.key(&mut state, ListKey::Down);
        assert_eq!(state.highlighted, Some(1));
        let p = list(400.0).place(&Metrics, &mut state);
        assert_eq!(state.scroll, 0.0);
        assert!(matches!(
            &p.widget.ops[0],
            Op::ClipRect {
                width: 35.0,
                height: 400.0,
                ..
            }
        ));
        let p = list(40.0).place(&Metrics, &mut state);
        p.wheel(&mut state, 10000.0);
        assert_eq!(state.scroll, 160.0);
    }
}
