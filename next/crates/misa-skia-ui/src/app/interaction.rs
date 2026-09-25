//! Frame-local interaction geometry and persistent selection/focus state.
//! Cached groups use the same coordinates for paint and hit testing; only this
//! owner rebases their row indices when placing them in a frame.
use super::{Control, text};
use misa_pixel_ui::{LaidOutRow, Op, Scene};
use misa_style::Style;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(super) struct Hit {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) control: Control,
}
impl Hit {
    pub(super) fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}
#[derive(Clone, Debug)]
pub(super) struct TextRow {
    pub(super) x: f32,
    pub(super) y: f32,
    /// The same local viewport used by the paint op and the hit rectangle.
    pub(super) width: f32,
    pub(super) geometry: Arc<LaidOutRow>,
}
impl TextRow {
    fn column(&self, x: f32) -> usize {
        self.geometry
            .advances
            .partition_point(|edge| *edge <= x - self.x)
            .saturating_sub(1)
    }
    pub(super) fn edge(&self, column: usize) -> f32 {
        let advances = &self.geometry.advances;
        advances[column.min(advances.len() - 1)].min(self.width)
    }
}
#[derive(Default)]
pub(super) struct InteractionMap {
    hits: Vec<Hit>,
    rows: Vec<TextRow>,
    focus: Option<Control>,
    selection: Option<((usize, usize), (usize, usize))>,
}

#[derive(Clone)]
pub(super) struct GroupGeometry {
    hits: Vec<Hit>,
    rows: Vec<TextRow>,
}
#[cfg(test)]
impl GroupGeometry {
    pub(super) fn rows(&self) -> &[TextRow] {
        &self.rows
    }
}

pub(super) enum PointerResult {
    None,
    SelectionChanged,
    Activate(Control),
}

impl InteractionMap {
    pub(super) fn focus(&self) -> Option<&Control> {
        self.focus.as_ref()
    }
    pub(super) fn focused(&self, control: &Control) -> bool {
        self.focus() == Some(control)
    }
    pub(super) fn set_focus(&mut self, focus: Option<Control>) {
        self.focus = focus;
    }
    pub(super) fn clear_selection(&mut self) {
        self.selection = None;
    }
    pub(super) fn select_all(&mut self) {
        if !self.rows.is_empty() {
            self.selection = Some(((0, 0), (self.rows.len() - 1, usize::MAX)));
        }
    }
    pub(super) fn next_focus(&mut self, backward: bool) {
        let controls: Vec<_> = self
            .hits
            .iter()
            .filter(|hit| !matches!(hit.control, Control::Text(_)))
            .map(|hit| hit.control.clone())
            .collect();
        if controls.is_empty() {
            return;
        }
        let current = controls
            .iter()
            .position(|control| Some(control) == self.focus());
        let next = match (current, backward) {
            (Some(index), true) => (index + controls.len() - 1) % controls.len(),
            (Some(index), false) => (index + 1) % controls.len(),
            (None, true) => controls.len() - 1,
            (None, false) => 0,
        };
        self.focus = Some(controls[next].clone());
    }
    pub(super) fn pointer(&mut self, x: f32, y: f32, dragging: bool) -> PointerResult {
        let Some(hit) = self.hits.iter().rev().find(|hit| hit.contains(x, y)) else {
            return PointerResult::None;
        };
        if let Control::Text(index) = &hit.control {
            // A text hit is always registered together with its measured row.
            let row = &self.rows[*index];
            let point = (*index, row.column(x));
            if dragging {
                if let Some((_, head)) = &mut self.selection {
                    *head = point;
                    return PointerResult::SelectionChanged;
                }
            } else {
                self.selection = Some((point, point));
                self.focus = None;
                return PointerResult::SelectionChanged;
            }
            return PointerResult::None;
        }
        if dragging {
            return PointerResult::None;
        }
        let control = hit.control.clone();
        self.selection = None;
        self.focus = Some(control.clone());
        PointerResult::Activate(control)
    }
    pub(super) fn begin_frame(&mut self) {
        self.hits.clear();
        self.rows.clear();
    }
    pub(super) fn clear_hits(&mut self) {
        self.hits.clear();
    }
    /// Scroll/repaint preserves selection, but a changed view can retire its rows.
    pub(super) fn finish_frame(&mut self) {
        if self
            .selection
            .is_some_and(|(anchor, head)| anchor.0 >= self.rows.len() || head.0 >= self.rows.len())
        {
            self.selection = None;
        }
    }
    pub(super) fn take_group(&mut self) -> GroupGeometry {
        GroupGeometry {
            hits: std::mem::take(&mut self.hits),
            rows: std::mem::take(&mut self.rows),
        }
    }
    pub(super) fn restore_group(&mut self, outer: GroupGeometry) -> GroupGeometry {
        let local = self.take_group();
        self.hits = outer.hits;
        self.rows = outer.rows;
        local
    }
    pub(super) fn place_group(&mut self, group: &GroupGeometry, x: f32, y: f32) {
        let base = self.rows.len();
        self.rows.extend(group.rows.iter().map(|row| TextRow {
            x: row.x + x,
            y: row.y + y,
            width: row.width,
            geometry: Arc::clone(&row.geometry),
        }));
        self.hits.extend(group.hits.iter().map(|hit| {
            let mut hit = hit.clone();
            hit.x += x;
            hit.y += y;
            if let Control::Text(row) = &mut hit.control {
                *row += base;
            }
            hit
        }));
    }
    pub(super) fn add_hit(&mut self, hit: Hit) {
        self.hits.push(hit);
    }
    pub(super) fn add_row(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        geometry: LaidOutRow,
    ) {
        let index = self.rows.len();
        self.add_hit(Hit {
            x,
            y,
            width,
            height,
            control: Control::Text(index),
        });
        self.rows.push(TextRow {
            x,
            y,
            width,
            geometry: Arc::new(geometry),
        });
    }
    pub(super) fn selected_text(&self) -> String {
        let Some((a, b)) = self.selection else {
            return String::new();
        };
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        (start.0..=end.0.min(self.rows.len().saturating_sub(1)))
            .filter_map(|index| {
                self.rows.get(index).map(|row| {
                    let from = if index == start.0 { start.1 } else { 0 };
                    let to = if index == end.0 { end.1 } else { usize::MAX };
                    row.geometry
                        .text
                        .chars()
                        .skip(from)
                        .take(to.saturating_sub(from))
                        .collect::<String>()
                })
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub(super) fn paint_selection(
        &self,
        scene: &mut Scene,
        line_height: f32,
        selection_style: Style,
    ) {
        let Some((a, b)) = self.selection else {
            return;
        };
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        for (index, row) in self
            .rows
            .iter()
            .enumerate()
            .skip(start.0)
            .take(end.0.saturating_sub(start.0) + 1)
        {
            let from = if index == start.0 { start.1 } else { 0 };
            let to = if index == end.0 { end.1 } else { usize::MAX };
            let from = from.min(row.geometry.text.chars().count());
            let to = to.min(row.geometry.text.chars().count());
            if to <= from {
                continue;
            }
            let mut selected = vec![Op::Rect {
                x: row.x + row.edge(from),
                y: row.y,
                width: row.edge(to) - row.edge(from),
                height: line_height,
                style: selection_style,
            }];
            for (style, run, start) in &row.geometry.runs {
                let end = start + run.chars().count();
                let left = from.max(*start);
                let right = to.min(end);
                if left < right {
                    let value: String = run.chars().skip(left - start).take(right - left).collect();
                    selected.push(text(row.x + row.edge(left), row.y, &value, *style));
                }
            }
            scene.ops.push(Op::ClipRect {
                x: row.x,
                y: row.y,
                width: row.width,
                height: line_height,
                ops: Arc::new(selected),
            });
        }
    }
    /// Lookup only: hosts need a pointer target, not mutable access to frame maps.
    pub(super) fn control_center(&self, control: &Control) -> Option<(f32, f32)> {
        self.hits
            .iter()
            .find(|hit| &hit.control == control)
            .map(|hit| (hit.x + hit.width / 2.0, hit.y + hit.height / 2.0))
    }
    #[cfg(test)]
    pub(super) fn rows(&self) -> &[TextRow] {
        &self.rows
    }
    #[cfg(test)]
    pub(super) fn hits(&self) -> &[Hit] {
        &self.hits
    }
    #[cfg(test)]
    pub(super) fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        self.selection
    }
}
