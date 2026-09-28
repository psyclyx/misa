//! Non-editable, protocol-free combo box. The caller owns the selected value and popup state.
use crate::fit::{self, Fit, Fitted, Insets};
use crate::{ContextMenu, MenuEntries, MenuEntry, MenuKey, MenuState, Op, Rect, TextMetrics};
use misa_style::Style;

pub struct ComboOption<Id> {
    pub value: Id,
    pub label: String,
    pub enabled: bool,
}
impl<Id> MenuEntries<Id> for [ComboOption<Id>] {
    fn len(&self) -> usize {
        <[ComboOption<Id>]>::len(self)
    }
    fn entry(&self, index: usize) -> Option<MenuEntry<'_, Id>> {
        self.get(index).map(|o| MenuEntry::Action {
            id: &o.value,
            label: &o.label,
            enabled: o.enabled,
        })
    }
}
impl<Id> MenuEntries<Id> for Vec<ComboOption<Id>> {
    fn len(&self) -> usize {
        self.as_slice().len()
    }
    fn entry(&self, index: usize) -> Option<MenuEntry<'_, Id>> {
        self.as_slice().entry(index)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ComboState {
    pub open: bool,
    pub menu: MenuState,
    pub menu_width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComboResult<Id> {
    Ignored,
    Handled,
    Selected(Id),
}

pub struct ComboBox<'a, 'b, Id> {
    pub options: &'a dyn MenuEntries<Id>,
    pub selected: Option<&'b Id>,
    pub bounds: Rect,
    pub viewport: Rect,
    pub font_size: f32,
    pub row_height: f32,
    pub focused: bool,
    pub enabled: bool,
    pub background: Style,
    pub foreground: Style,
    pub muted: Style,
    pub border: Style,
    pub highlight: Style,
}

pub struct PlacedComboBox<'a, Id> {
    pub bounds: Rect,
    pub content_width: f32,
    pub ops: Vec<Op>,
    options: &'a dyn MenuEntries<Id>,
    viewport: Rect,
    font_size: f32,
    row_height: f32,
    background: Style,
    foreground: Style,
    muted: Style,
    highlight: Style,
    enabled: bool,
    selected_index: Option<usize>,
}

impl<'a, Id: Clone + PartialEq> ComboBox<'a, '_, Id> {
    pub fn place(&self, metrics: &dyn TextMetrics) -> PlacedComboBox<'a, Id> {
        let bounds = self.bounds;
        let selected_index = self.selected.and_then(|value| (0..self.options.len()).position(|i| matches!(self.options.entry(i), Some(MenuEntry::Action { id, .. }) if id == value)));
        let label = selected_index
            .and_then(|i| match self.options.entry(i) {
                Some(MenuEntry::Action { label, .. }) => Some(label),
                _ => None,
            })
            .or_else(|| self.selected.map(|_| "Unavailable selection"))
            .unwrap_or("");
        // The label shares its box with a reserved 29px arrow affordance.
        let content = fit::measure_line(label, self.font_size, metrics);
        let fitted = Fitted::new(bounds, Fit::Fill, Insets::horizontal(7.0, 29.0), content);
        let arrow = Fitted::new(
            Rect {
                x: (bounds.x + bounds.width - 20.0).max(bounds.x),
                y: bounds.y,
                width: bounds.width.min(20.0).max(0.0),
                height: bounds.height,
            },
            Fit::Fill,
            Insets::ZERO,
            content,
        );
        let ops = vec![
            Op::Rect {
                x: bounds.x - 1.0,
                y: bounds.y - 1.0,
                width: bounds.width + 2.0,
                height: bounds.height + 2.0,
                style: if self.focused {
                    self.highlight
                } else {
                    self.border
                },
            },
            Op::Rect {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
                style: self.background,
            },
            fit::text_line(
                fitted.inner,
                label.into(),
                self.font_size,
                if self.enabled {
                    self.foreground
                } else {
                    self.muted
                },
                metrics,
            ),
            fit::text_line(
                arrow.inner,
                "▾".into(),
                self.font_size,
                self.foreground,
                metrics,
            ),
        ];
        PlacedComboBox {
            bounds,
            content_width: content.width,
            ops,
            options: self.options,
            viewport: self.viewport,
            font_size: self.font_size,
            row_height: self.row_height,
            background: self.background,
            foreground: self.foreground,
            muted: self.muted,
            highlight: self.highlight,
            enabled: self.enabled,
            selected_index,
        }
    }
}

impl<Id: Clone> PlacedComboBox<'_, Id> {
    fn menu(&self) -> ContextMenu<'_, Id> {
        ContextMenu {
            items: self.options,
            anchor: (self.bounds.x, self.bounds.y + self.bounds.height),
            viewport: self.viewport,
            font_size: self.font_size,
            row_height: self.row_height,
            background: self.background,
            foreground: self.foreground,
            muted: self.muted,
            highlight: self.highlight,
        }
    }
    fn open(&self, metrics: &dyn TextMetrics, state: &mut ComboState) {
        if !self.enabled {
            return;
        }
        state.open = true;
        state.menu = MenuState::default();
        state.menu_width = self.menu().measured_width(metrics);
        state.menu.highlighted = self.selected_index.filter(|i| {
            matches!(
                self.options.entry(*i),
                Some(MenuEntry::Action { enabled: true, .. })
            )
        });
        // Keep the selected row visible even when opening a long list at the edge.
        if let Some(i) = state.menu.highlighted {
            let visible = (self.viewport.height / self.row_height).floor().max(1.0) as usize;
            state.menu.scroll = i.saturating_sub(visible - 1) as f32 * self.row_height;
        }
    }
    pub fn popup(&self, metrics: &dyn TextMetrics, state: &mut ComboState) -> Vec<Op> {
        if !state.open {
            return vec![];
        }
        self.menu()
            .place_with_width(metrics, &mut state.menu, state.menu_width)
            .ops
    }
    pub fn click(
        &self,
        metrics: &dyn TextMetrics,
        state: &mut ComboState,
        x: f32,
        y: f32,
    ) -> ComboResult<Id> {
        if state.open {
            let result = self
                .menu()
                .place_with_width(metrics, &mut state.menu, state.menu_width)
                .click(x, y);
            return self.finish(state, result);
        }
        if self.enabled && self.bounds.contains(x, y) {
            self.open(metrics, state);
            ComboResult::Handled
        } else {
            ComboResult::Ignored
        }
    }
    fn finish(&self, state: &mut ComboState, result: Option<Option<Id>>) -> ComboResult<Id> {
        match result {
            Some(Some(id)) => {
                state.open = false;
                ComboResult::Selected(id)
            }
            Some(None) => {
                state.open = false;
                ComboResult::Handled
            }
            None => ComboResult::Handled,
        }
    }
    pub fn key(
        &self,
        metrics: &dyn TextMetrics,
        state: &mut ComboState,
        key: MenuKey,
    ) -> ComboResult<Id> {
        if !state.open {
            if key == MenuKey::Escape || !self.enabled {
                return ComboResult::Ignored;
            }
            self.open(metrics, state);
            if key == MenuKey::Enter {
                return ComboResult::Handled;
            }
        }
        let result = self
            .menu()
            .place_with_width(metrics, &mut state.menu, state.menu_width)
            .key(&mut state.menu, key);
        self.finish(state, result)
    }
    pub fn wheel(&self, metrics: &dyn TextMetrics, state: &mut ComboState, delta: f32) {
        if state.open {
            self.menu()
                .place_with_width(metrics, &mut state.menu, state.menu_width)
                .wheel(&mut state.menu, delta);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure(&self, text: &str, size: f32) -> f32 {
            text.len() as f32 * size
        }
        fn advances(&self, text: &str, size: f32) -> Vec<f32> {
            (0..=text.len()).map(|i| i as f32 * size).collect()
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
    #[test]
    fn ten_thousand_rows_measure_only_on_open_and_borrow_updated_content() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counting(AtomicUsize);
        impl TextMetrics for Counting {
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
        let metrics = Counting(AtomicUsize::new(0));
        let mut options: Vec<_> = (0..10_000)
            .map(|i| ComboOption {
                value: i,
                label: format!("Row {i}"),
                enabled: true,
            })
            .collect();
        let selected = 9999;
        fn build<'a>(
            options: &'a Vec<ComboOption<i32>>,
            selected: &i32,
            viewport: Rect,
            metrics: &dyn TextMetrics,
        ) -> PlacedComboBox<'a, i32> {
            ComboBox {
                options,
                selected: Some(&selected),
                bounds: Rect {
                    x: 90.0,
                    y: 5.0,
                    width: 10.0,
                    height: 20.0,
                },
                viewport,
                font_size: 12.0,
                row_height: 20.0,
                focused: true,
                enabled: true,
                background: Style::default(),
                foreground: Style::default(),
                muted: Style::default(),
                border: Style::default(),
                highlight: Style::default(),
            }
            .place(metrics)
        }
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 60.0,
        };
        let mut state = ComboState::default();
        for _ in 0..3 {
            let _ = build(&options, &selected, viewport, &metrics);
        }
        assert!(metrics.0.load(Ordering::Relaxed) < 10);
        let widget = build(&options, &selected, viewport, &metrics);
        assert_eq!(
            widget.key(&metrics, &mut state, MenuKey::Enter),
            ComboResult::Handled
        );
        assert!(metrics.0.load(Ordering::Relaxed) >= 10_000);
        metrics.0.store(0, Ordering::Relaxed);
        for _ in 0..5 {
            let popup = build(&options, &selected, viewport, &metrics).popup(&metrics, &mut state);
            assert!(matches!(&popup[0], Op::ClipRect { width: 100.0, .. }));
        }
        assert!(
            metrics.0.load(Ordering::Relaxed) < 20,
            "steady-state redraw must not measure the menu"
        );
        let resized = Rect {
            width: 35.0,
            ..viewport
        };
        assert!(matches!(
            &build(&options, &selected, resized, &metrics).popup(&metrics, &mut state)[0],
            Op::ClipRect { width: 35.0, .. }
        ));
        assert_eq!(
            build(&options, &selected, resized, &metrics).key(&metrics, &mut state, MenuKey::Up),
            ComboResult::Handled
        );
        assert_eq!(state.menu.highlighted, Some(9998));
        assert_eq!(
            build(&options, &selected, resized, &metrics).key(
                &metrics,
                &mut state,
                MenuKey::Escape
            ),
            ComboResult::Handled
        );
        assert!(!state.open);
        options[9999].label = "Updated".into();
        assert!(
            build(&options, &selected, resized, &metrics)
                .ops
                .iter()
                .any(|op| matches!(op, Op::ClipRect { ops, .. }
            if ops.iter().any(|op| matches!(op, Op::Text { text, .. } if text == "Updated"))))
        );
        let widget = build(&options, &selected, resized, &metrics);
        assert_eq!(
            widget.click(&metrics, &mut state, 95.0, 10.0),
            ComboResult::Handled
        );
        assert_eq!(
            widget.click(&metrics, &mut state, 200.0, 200.0),
            ComboResult::Handled
        );
        assert!(!state.open);
    }
    #[test]
    fn stable_values_disabled_navigation_and_edge_scroll() {
        let options: Vec<_> = (0..30)
            .map(|i| ComboOption {
                value: format!("id{i}"),
                label: format!("Label {i}"),
                enabled: i != 28,
            })
            .collect();
        let selected = "id29".to_string();
        let combo = ComboBox {
            options: &options,
            selected: Some(&selected),
            bounds: Rect {
                x: 80.0,
                y: 10.0,
                width: 20.0,
                height: 25.0,
            },
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 90.0,
            },
            font_size: 12.0,
            row_height: 25.0,
            focused: true,
            enabled: true,
            background: Style::default(),
            foreground: Style::default(),
            muted: Style::default(),
            border: Style::default(),
            highlight: Style::default(),
        }
        .place(&Metrics);
        let mut state = ComboState::default();
        assert_eq!(combo.content_width, 8.0 * 12.0);
        assert_eq!(
            combo.click(&Metrics, &mut state, 90.0, 20.0),
            ComboResult::Handled
        );
        assert_eq!(state.menu.highlighted, Some(29));
        assert!(state.menu.scroll > 0.0);
        let popup = combo.popup(&Metrics, &mut state);
        assert!(matches!(
            popup[0],
            Op::ClipRect {
                x: 0.0,
                width: 100.0,
                height: 90.0,
                ..
            }
        ));
        assert_eq!(
            combo.click(&Metrics, &mut state, 2.0, 55.0),
            ComboResult::Handled
        );
        assert!(state.open); // disabled row cannot commit
        assert_eq!(
            combo.key(&Metrics, &mut state, MenuKey::Up),
            ComboResult::Handled
        );
        assert_eq!(state.menu.highlighted, Some(27));
        assert_eq!(
            combo.key(&Metrics, &mut state, MenuKey::Enter),
            ComboResult::Selected("id27".into())
        );
        assert!(!state.open);
    }
}
