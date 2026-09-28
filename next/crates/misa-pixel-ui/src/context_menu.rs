//! Local context menu: caller owns the items, focus and lifetime of the overlay.
use crate::fit::{self, Fit, Fitted, Insets, Size};
use crate::{Op, Rect, TextMetrics};
use misa_style::Style;
/// Menu entries keep their labels away from the menu edges.
const MENU_INSETS: Insets = Insets::horizontal(12.0, 12.0);

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
/// Borrowed indexed rows allow large menus to paint without copying their contents.
pub enum MenuEntry<'a, Id> {
    Action {
        id: &'a Id,
        label: &'a str,
        enabled: bool,
    },
    Separator,
}
pub trait MenuEntries<Id> {
    fn len(&self) -> usize;
    fn entry(&self, index: usize) -> Option<MenuEntry<'_, Id>>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
impl<Id> MenuEntries<Id> for [MenuItem<Id>] {
    fn len(&self) -> usize {
        <[MenuItem<Id>]>::len(self)
    }
    fn entry(&self, index: usize) -> Option<MenuEntry<'_, Id>> {
        self.get(index).map(|item| match item {
            MenuItem::Action { id, label, enabled } => MenuEntry::Action {
                id,
                label,
                enabled: *enabled,
            },
            MenuItem::Separator => MenuEntry::Separator,
        })
    }
}
impl<Id> MenuEntries<Id> for Vec<MenuItem<Id>> {
    fn len(&self) -> usize {
        self.as_slice().len()
    }
    fn entry(&self, index: usize) -> Option<MenuEntry<'_, Id>> {
        self.as_slice().entry(index)
    }
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
    pub items: &'a dyn MenuEntries<Id>,
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
    items: &'a dyn MenuEntries<Id>,
    row_height: f32,
    scroll: f32,
}
impl<Id> ContextMenu<'_, Id> {
    /// Measure only when items change; callers with stable content can reuse this width.
    pub fn measured_width(&self, metrics: &dyn TextMetrics) -> f32 {
        let content = (0..self.items.len())
            .filter_map(|i| match self.items.entry(i) {
                Some(MenuEntry::Action { label, .. }) => {
                    Some(fit::measure_line(label, self.font_size, metrics).width)
                }
                _ => None,
            })
            .fold(0.0_f32, f32::max);
        // The natural width, before the viewport ever caps it: callers measure
        // with placeholder geometry and reuse the number at placement time.
        fit::natural(
            Size {
                width: content,
                height: 0.0,
            },
            MENU_INSETS,
        )
        .width
    }
    pub fn place<'a>(
        &'a self,
        metrics: &dyn TextMetrics,
        state: &mut MenuState,
    ) -> PlacedMenu<'a, Id> {
        self.place_with_width(metrics, state, self.measured_width(metrics))
    }
    pub fn place_with_width<'a>(
        &'a self,
        metrics: &dyn TextMetrics,
        state: &mut MenuState,
        measured_width: f32,
    ) -> PlacedMenu<'a, Id> {
        assert!(self.row_height > 0.0);
        let v = self.viewport;
        let width = measured_width.max(72.0).min(v.width.max(0.0));
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
            if state.highlighted == Some(i) {
                ops.push(Op::Rect {
                    x: bounds.x,
                    y,
                    width,
                    height: self.row_height,
                    style: self.highlight,
                });
            }
            match self.items.entry(i).expect("visible row") {
                MenuEntry::Separator => ops.push(Op::Rect {
                    x: bounds.x + 6.0,
                    y: y + self.row_height / 2.0,
                    width: (width - 12.0).max(0.0),
                    height: 1.0,
                    style: self.muted,
                }),
                MenuEntry::Action { label, enabled, .. } => {
                    // The menu precomputed its width from its entries; rows
                    // never re-measure, steady-state redraws included.
                    let fitted = Fitted::new(
                        Rect {
                            x: bounds.x,
                            y,
                            width,
                            height: self.row_height,
                        },
                        Fit::Fill,
                        MENU_INSETS,
                        Size::ZERO,
                    );
                    ops.push(fit::text_line(
                        fitted.inner,
                        label.into(),
                        self.font_size,
                        if enabled { self.foreground } else { self.muted },
                        metrics,
                    ))
                }
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
fn selectable<Id>(items: &dyn MenuEntries<Id>, index: usize) -> bool {
    matches!(
        items.entry(index),
        Some(MenuEntry::Action { enabled: true, .. })
    )
}
impl<Id: Clone> PlacedMenu<'_, Id> {
    /// `Some(None)` dismisses on an outside click; `Some(Some(id))` activates.
    pub fn click(&self, x: f32, y: f32) -> Option<Option<Id>> {
        if !self.bounds.contains(x, y) {
            return Some(None);
        }
        let index = ((y - self.bounds.y + self.scroll) / self.row_height) as usize;
        match self.items.entry(index) {
            Some(MenuEntry::Action {
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
            return state.highlighted.and_then(|i| match self.items.entry(i) {
                Some(MenuEntry::Action {
                    id, enabled: true, ..
                }) => Some(Some(id.clone())),
                _ => None,
            });
        }
        let len = self.items.len();
        if len == 0 {
            return None;
        }
        let (start, backward) = match key {
            MenuKey::Home => (0, false),
            MenuKey::End => (len - 1, true),
            MenuKey::Down => (state.highlighted.map_or(0, |i| (i + 1) % len), false),
            MenuKey::Up => (
                state.highlighted.map_or(len - 1, |i| (i + len - 1) % len),
                true,
            ),
            _ => unreachable!(),
        };
        let Some(i) = (0..len)
            .map(|n| {
                if backward {
                    (start + len - n) % len
                } else {
                    (start + n) % len
                }
            })
            .find(|i| selectable(self.items, *i))
        else {
            return None;
        };
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
        let width = menu.measured_width(&Metrics);
        let p = menu.place_with_width(&Metrics, &mut state, width);
        assert_eq!(
            p.bounds,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 60.0
            }
        );
        assert_eq!(p.click(2.0, 25.0), None);
        assert_eq!(p.click(40.0, 25.0), Some(None));
        p.key(&mut state, MenuKey::Down);
        assert_eq!(state.highlighted, Some(2));
        assert_eq!(p.key(&mut state, MenuKey::Enter), Some(Some(1)));
        p.key(&mut state, MenuKey::End);
        assert_eq!(state.highlighted, Some(20));
        assert_eq!(state.scroll, 360.0);
        let p = menu.place_with_width(&Metrics, &mut state, width);
        assert_eq!(p.click(2.0, 59.0), Some(Some(19)));
        assert_eq!(p.key(&mut state, MenuKey::Escape), Some(None));
    }
}
