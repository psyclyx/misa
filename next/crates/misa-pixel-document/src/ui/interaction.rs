//! Frame-local interaction geometry and persistent semantic selection.
use super::{Control, text};
use misa_pixel_ui::{FieldViewport, LaidOutRow, Op, PlacedField, Scene};
use misa_style::Style;
use std::collections::BTreeSet;
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
    pub(super) width: f32,
    pub(super) geometry: Arc<LaidOutRow>,
    owner: String,
    /// Scalar offset in the owner's logical text (not a frame row number).
    source_start: usize,
    prefix: usize,
    /// Soft wrap versus explicit source break / separate logical row.
    continued: bool,
    break_before: bool,
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
    fn point(&self, column: usize) -> (String, usize) {
        (
            self.owner.clone(),
            self.source_start
                + column
                    .min(self.geometry.text.chars().count())
                    .saturating_sub(self.prefix),
        )
    }
}
#[derive(Clone, Debug)]
struct SelectedPart {
    owner: String,
    start: usize,
    end: usize,
}
#[derive(Clone, Debug)]
struct Selection {
    anchor: (String, usize),
    anchor_at_end: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    head: (String, usize),
    parts: Vec<SelectedPart>,
    /// The selected content is independent of viewport residency and reflow.
    text: String,
}
#[derive(Default)]
pub(super) struct InteractionMap {
    hits: Vec<Hit>,
    rows: Vec<TextRow>,
    focus: Option<Control>,
    expanded: BTreeSet<String>,
    selection: Option<Selection>,
    owner: String,
    cursor: usize,
    continues: bool,
}
#[derive(Clone)]
pub(super) struct GroupGeometry {
    hits: Vec<Hit>,
    rows: Vec<TextRow>,
    owner: String,
    cursor: usize,
    continues: bool,
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
    pub(super) fn is_expanded(&self, id: &str) -> bool {
        self.expanded.contains(id)
    }
    pub(super) fn toggle_disclosure(&mut self, id: &str) {
        if !self.expanded.remove(id) {
            self.expanded.insert(id.to_owned());
        }
    }
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
    pub(super) fn retire(&mut self, ids: &BTreeSet<String>) {
        if self.selection.as_ref().is_some_and(|s| {
            s.parts.iter().any(|p| {
                ids.contains(&p.owner)
                    || ids.iter().any(|id| {
                        p.owner.starts_with(&format!("\0row:{id}:"))
                            || p.owner == format!("\0end:{id}")
                    })
            })
        }) {
            self.clear_selection();
        }
    }
    pub(super) fn select_all(&mut self) {
        if !self.rows.is_empty() {
            self.set_selection((0, 0), (self.rows.len() - 1, usize::MAX));
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
            let point = (*index, self.rows[*index].column(x));
            if dragging {
                if let Some(selection) = &self.selection {
                    let anchor = &selection.anchor;
                    let candidates = self.rows.iter().enumerate().filter_map(|(i, row)| {
                        let length = row.geometry.text.chars().count().saturating_sub(row.prefix);
                        (row.owner == anchor.0
                            && anchor.1 >= row.source_start
                            && anchor.1 <= row.source_start + length)
                            .then_some((i, row.prefix + anchor.1.saturating_sub(row.source_start)))
                    });
                    let anchor_row = if selection.anchor_at_end {
                        candidates.into_iter().next()
                    } else {
                        candidates.into_iter().last()
                    };
                    let Some((row_index, column)) = anchor_row else {
                        return PointerResult::None;
                    };
                    self.set_selection((row_index, column), point);
                    return PointerResult::SelectionChanged;
                }
            } else {
                self.set_selection(point, point);
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
    /// Offscreen owners are not retired: only document changes can invalidate
    /// their semantic selection. Keep the frame boundary explicit for callers.
    pub(super) fn finish_frame(&mut self) {}
    pub(super) fn begin_owner(&mut self, owner: &str) {
        self.owner = owner.to_owned();
        self.cursor = 0;
        self.continues = false;
    }
    pub(super) fn continue_row(&mut self, skipped: usize) {
        self.cursor += skipped;
        self.continues = true;
    }
    pub(super) fn source_cursor(&self) -> usize {
        self.cursor
    }
    /// Table cells have independent logical ranges even when their visual rows interleave.
    pub(super) fn next_source(&mut self, offset: usize, continued: bool) {
        self.cursor = if continued {
            offset
        } else {
            offset.saturating_sub(1)
        };
        self.continues = continued || offset == 0;
    }
    pub(super) fn finish_source(&mut self, offset: usize) {
        self.cursor = offset;
        self.continues = false;
    }
    pub(super) fn take_group(&mut self) -> GroupGeometry {
        GroupGeometry {
            hits: std::mem::take(&mut self.hits),
            rows: std::mem::take(&mut self.rows),
            owner: std::mem::take(&mut self.owner),
            cursor: std::mem::take(&mut self.cursor),
            continues: std::mem::take(&mut self.continues),
        }
    }
    pub(super) fn restore_group(&mut self, outer: GroupGeometry) -> GroupGeometry {
        let local = self.take_group();
        self.hits = outer.hits;
        self.rows = outer.rows;
        self.owner = outer.owner;
        self.cursor = outer.cursor;
        self.continues = outer.continues;
        local
    }
    pub(super) fn place_group(&mut self, group: &GroupGeometry, x: f32, y: f32) {
        let base = self.rows.len();
        self.rows.extend(group.rows.iter().map(|row| TextRow {
            x: row.x + x,
            y: row.y + y,
            width: row.width,
            geometry: Arc::clone(&row.geometry),
            owner: row.owner.clone(),
            source_start: row.source_start,
            prefix: row.prefix,
            continued: row.continued,
            break_before: row.break_before,
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
    pub(super) fn place_field(
        &mut self,
        scene: &mut Scene,
        field: PlacedField<Control>,
    ) -> (Control, FieldViewport) {
        self.add_hit(Hit {
            x: field.bounds.x,
            y: field.bounds.y,
            width: field.bounds.width,
            height: field.bounds.height,
            control: field.id.clone(),
        });
        scene.ops.extend(field.ops);
        (field.id, field.viewport)
    }
    pub(super) fn add_row(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        geometry: LaidOutRow,
        prefix: usize,
    ) {
        let index = self.rows.len();
        self.add_hit(Hit {
            x,
            y,
            width,
            height,
            control: Control::Text(index),
        });
        let continued = self.continues;
        let previous = self.rows.last().filter(|row| row.owner == self.owner);
        let before = previous
            .map(|row| {
                row.source_start + row.geometry.text.chars().count().saturating_sub(row.prefix)
            })
            .unwrap_or(0);
        if !continued && self.cursor > 0 {
            self.cursor += 1;
        }
        self.continues = false;
        let source_start = self.cursor;
        let break_before = previous.is_some() && source_start > before;
        self.cursor += geometry.text.chars().count().saturating_sub(prefix);
        self.rows.push(TextRow {
            x,
            y,
            width,
            geometry: Arc::new(geometry),
            owner: self.owner.clone(),
            source_start,
            prefix,
            continued,
            break_before,
        });
    }
    fn set_selection(&mut self, a: (usize, usize), b: (usize, usize)) {
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let mut parts: Vec<SelectedPart> = Vec::new();
        let mut text = String::new();
        for index in start.0..=end.0 {
            let Some(row) = self.rows.get(index) else {
                return;
            };
            let from =
                if index == start.0 { start.1 } else { 0 }.min(row.geometry.text.chars().count());
            let to = if index == end.0 { end.1 } else { usize::MAX }
                .min(row.geometry.text.chars().count());
            if index > start.0 && (!row.continued || row.break_before) {
                text.push('\n');
            }
            let from = if row.continued {
                from.max(row.prefix)
            } else {
                from
            };
            text.extend(
                row.geometry
                    .text
                    .chars()
                    .skip(from)
                    .take(to.saturating_sub(from)),
            );
            let left = row.source_start + from.saturating_sub(row.prefix);
            let right = row.source_start + to.saturating_sub(row.prefix);
            if let Some(part) = parts
                .last_mut()
                .filter(|part| part.owner == row.owner && left >= part.start && left <= part.end)
            {
                part.end = right;
            } else {
                parts.push(SelectedPart {
                    owner: row.owner.clone(),
                    start: left,
                    end: right,
                });
            }
        }
        self.selection = Some(Selection {
            anchor: self.rows[a.0].point(a.1),
            anchor_at_end: a.1 > 0 && a.1 >= self.rows[a.0].geometry.text.chars().count(),
            head: self.rows[b.0].point(b.1),
            parts,
            text,
        });
    }
    pub(super) fn selected_text(&self) -> String {
        self.selection
            .as_ref()
            .map_or_else(String::new, |s| s.text.clone())
    }
    pub(super) fn paint_selection(
        &self,
        scene: &mut Scene,
        line_height: f32,
        selection_style: Style,
    ) {
        let Some(selection) = &self.selection else {
            return;
        };
        for row in &self.rows {
            let Some(part) = selection.parts.iter().find(|p| {
                p.owner == row.owner
                    && p.end > row.source_start
                    && p.start
                        < row.source_start
                            + row.geometry.text.chars().count().saturating_sub(row.prefix)
            }) else {
                continue;
            };
            let len = row.geometry.text.chars().count();
            if part.end <= row.source_start
                || part.start >= row.source_start + len.saturating_sub(row.prefix)
            {
                continue;
            }
            let from = if part.start <= row.source_start {
                0
            } else {
                row.prefix + part.start - row.source_start
            }
            .min(len);
            let to = if part.end >= row.source_start + len.saturating_sub(row.prefix) {
                len
            } else {
                row.prefix + part.end.saturating_sub(row.source_start)
            }
            .min(len);
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
    pub(super) fn hit_at(&self, x: f32, y: f32) -> Option<Control> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.contains(x, y))
            .map(|hit| hit.control.clone())
    }
    pub(super) fn control_bounds(&self, control: &Control) -> Option<misa_pixel_ui::Rect> {
        self.hits
            .iter()
            .find(|hit| &hit.control == control)
            .map(|hit| misa_pixel_ui::Rect {
                x: hit.x,
                y: hit.y,
                width: hit.width,
                height: hit.height,
            })
    }
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
    pub(super) fn selection(&self) -> Option<((String, usize), (String, usize))> {
        self.selection
            .as_ref()
            .map(|s| (s.anchor.clone(), s.head.clone()))
    }
}
