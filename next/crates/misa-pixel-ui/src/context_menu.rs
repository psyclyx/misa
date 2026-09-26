//! Local context menu: caller owns the items, focus and lifetime of the overlay.
use crate::{Op, Rect, TextMetrics};
use misa_style::Style;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem<Id> {
    Action {
        id: Id,
        label: String,
        enabled: bool,
    },
    Separator,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Escape,
    Up,
    Down,
    Home,
    End,
    Enter,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MenuState {
    pub highlighted: Option<usize>,
    pub scroll: f32,
}
pub struct ContextMenu<'a, Id> {
    pub items: &'a [MenuItem<Id>],
    pub anchor: (f32, f32),
    pub viewport: Rect,
    pub font_size: f32,
    pub row_height: f32,
    pub background: Style,
    pub foreground: Style,
    pub muted: Style,
    pub highlight: Style,
}
pub struct PlacedMenu<'a, Id> {
    pub bounds: Rect,
    pub ops: Vec<Op>,
    items: &'a [MenuItem<Id>],
    row_height: f32,
    scroll: f32,
}
impl<Id> ContextMenu<'_, Id> {
    pub fn place<'a>(
        &'a self,
        metrics: &dyn TextMetrics,
        state: &mut MenuState,
    ) -> PlacedMenu<'a, Id> {
        assert!(self.row_height > 0.0);
        let v = self.viewport;
        let width = (self
            .items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { label, .. } => Some(metrics.measure(label, self.font_size)),
                MenuItem::Separator => None,
            })
            .fold(0.0_f32, f32::max)
            + 24.0)
            .max(72.0)
            .min(v.width.max(0.0));
        let height = (self.items.len() as f32 * self.row_height).min(v.height.max(0.0));
        let bounds = Rect {
            x: self.anchor.0.clamp(v.x, (v.x + v.width - width).max(v.x)),
            y: self.anchor.1.clamp(v.y, (v.y + v.height - height).max(v.y)),
            width,
            height,
        };
        if state
            .highlighted
            .is_some_and(|i| !selectable(self.items, i))
        {
            state.highlighted = None;
        }
        let max_scroll = (self.items.len() as f32 * self.row_height - height).max(0.0);
        state.scroll = state.scroll.clamp(0.0, max_scroll);
        let mut ops = vec![Op::Rect {
            x: bounds.x,
            y: bounds.y,
            width,
            height,
            style: self.background,
        }];
        let start = (state.scroll / self.row_height).floor() as usize;
        let end = ((state.scroll + height) / self.row_height).ceil() as usize;
        for i in start..end.min(self.items.len()) {
            let y = bounds.y + i as f32 * self.row_height - state.scroll;
            let row = Rect {
                x: bounds.x,
                y,
                width,
                height: self.row_height,
            };
            if state.highlighted == Some(i) {
                ops.push(Op::Rect {
                    x: row.x,
                    y,
                    width,
                    height: row.height,
                    style: self.highlight,
                });
            }
            match &self.items[i] {
                MenuItem::Separator => ops.push(Op::Rect {
                    x: row.x + 6.0,
                    y: y + row.height / 2.0,
                    width: (width - 12.0).max(0.0),
                    height: 1.0,
                    style: self.muted,
                }),
                MenuItem::Action { label, enabled, .. } => ops.push(Op::ClipRect {
                    x: row.x + 12.0,
                    y,
                    width: (width - 24.0).max(0.0),
                    height: row.height,
                    ops: Arc::new(vec![Op::Text {
                        x: row.x + 12.0,
                        y: y + ((row.height - metrics.line_metrics(self.font_size).line_height)
                            / 2.0)
                            .max(0.0),
                        size: self.font_size,
                        style: if *enabled {
                            self.foreground
                        } else {
                            self.muted
                        },
                        text: label.clone(),
                    }]),
                }),
            }
        }
        PlacedMenu {
            bounds,
            ops: vec![Op::ClipRect {
                x: bounds.x,
                y: bounds.y,
                width,
                height,
                ops: Arc::new(ops),
            }],
            items: self.items,
            row_height: self.row_height,
            scroll: state.scroll,
        }
    }
}
fn selectable<Id>(items: &[MenuItem<Id>], index: usize) -> bool {
    matches!(
        items.get(index),
        Some(MenuItem::Action { enabled: true, .. })
    )
}
impl<Id: Clone> PlacedMenu<'_, Id> {
    /// `Some(None)` dismisses on an outside click; `Some(Some(id))` activates.
    pub fn click(&self, x: f32, y: f32) -> Option<Option<Id>> {
        if !self.bounds.contains(x, y) {
            return Some(None);
        }
        let index = ((y - self.bounds.y + self.scroll) / self.row_height) as usize;
        match self.items.get(index) {
            Some(MenuItem::Action {
                id, enabled: true, ..
            }) => Some(Some(id.clone())),
            _ => None,
        }
    }
    pub fn wheel(&self, state: &mut MenuState, delta: f32) {
        if delta.is_finite() {
            state.scroll = (self.scroll + delta).clamp(
                0.0,
                (self.items.len() as f32 * self.row_height - self.bounds.height).max(0.0),
            );
        }
    }
    /// `Some(None)` dismisses; `Some(Some(id))` activates.
    pub fn key(&self, state: &mut MenuState, key: MenuKey) -> Option<Option<Id>> {
        if key == MenuKey::Escape {
            return Some(None);
        }
        if key == MenuKey::Enter {
            return state.highlighted.and_then(|i| match self.items.get(i) {
                Some(MenuItem::Action {
                    id, enabled: true, ..
                }) => Some(Some(id.clone())),
                _ => None,
            });
        }
        let indices: Vec<_> = (0..self.items.len())
            .filter(|i| selectable(self.items, *i))
            .collect();
        if indices.is_empty() {
            return None;
        }
        let pos = state
            .highlighted
            .and_then(|i| indices.iter().position(|n| *n == i));
        let next = match key {
            MenuKey::Home => 0,
            MenuKey::End => indices.len() - 1,
            MenuKey::Down => pos.map_or(0, |p| (p + 1) % indices.len()),
            MenuKey::Up => pos.map_or(indices.len() - 1, |p| {
                (p + indices.len() - 1) % indices.len()
            }),
            _ => unreachable!(),
        };
        let i = indices[next];
        state.highlighted = Some(i);
        let top = i as f32 * self.row_height;
        state.scroll = self
            .scroll
            .min(top)
            .max(top + self.row_height - self.bounds.height)
            .clamp(
                0.0,
                (self.items.len() as f32 * self.row_height - self.bounds.height).max(0.0),
            );
        None
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
            (0..=text.len()).map(|n| n as f32 * size).collect()
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
    fn narrow_edge_scroll_disabled_and_dismiss() {
        let mut items = vec![
            MenuItem::Separator,
            MenuItem::Action {
                id: 0,
                label: "disabled".into(),
                enabled: false,
            },
        ];
        items.extend((1..20).map(|i| MenuItem::Action {
            id: i,
            label: "long label".into(),
            enabled: true,
        }));
        let menu = ContextMenu {
            items: &items,
            anchor: (99.0, 99.0),
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 60.0,
            },
            font_size: 10.0,
            row_height: 20.0,
            background: Style::default(),
            foreground: Style::default(),
            muted: Style::default(),
            highlight: Style::default(),
        };
        let mut state = MenuState::default();
        let p = menu.place(&Metrics, &mut state);
        assert_eq!(
            p.bounds,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 60.0
            }
        );
        assert!(matches!(&p.ops[0], Op::ClipRect { width: 40.0, .. }));
        assert_eq!(p.click(2.0, 25.0), None);
        assert_eq!(p.click(40.0, 25.0), Some(None));
        p.key(&mut state, MenuKey::Down);
        assert_eq!(state.highlighted, Some(2));
        assert_eq!(p.key(&mut state, MenuKey::Enter), Some(Some(1)));
        p.key(&mut state, MenuKey::End);
        assert_eq!(state.highlighted, Some(20));
        assert_eq!(state.scroll, 360.0);
        let p = menu.place(&Metrics, &mut state);
        assert_eq!(p.click(2.0, 59.0), Some(Some(19)));
        assert_eq!(p.key(&mut state, MenuKey::Escape), Some(None));
    }
}
