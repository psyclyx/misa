//! Exact, sparse vertical flow. The source owns ordering and measurements; the
//! viewport owns only a scroll anchor, visible placements, and measured heights.
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;

/// IDs remain stable across edits. `previous`/`next` are indexed navigation,
/// not iterators over a materialized document. `measure` returns the exact height
/// at this width and style generation (including any nested bounded viewport).
/// Measurement may populate a retained display list, but must not place hits or
/// apply editor viewport effects; those belong to visible placement.
pub trait FlowSource {
    type Id: Clone + Eq + Hash;
    fn first(&self) -> Option<Self::Id>;
    fn last(&self) -> Option<Self::Id>;
    fn previous(&self, id: &Self::Id) -> Option<Self::Id>;
    fn next(&self, id: &Self::Id) -> Option<Self::Id>;
    fn contains(&self, id: &Self::Id) -> bool;
    fn measure(&mut self, id: &Self::Id, width: f32, style_generation: u64) -> f32;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constraints {
    pub width: f32,
    pub height: f32,
    pub style_generation: u64,
}

/// `local_y` is a point in an owner, held at `screen_y` across edits/resizes.
#[derive(Clone, Debug, PartialEq)]
pub enum FlowPosition<Id> {
    FollowTail,
    Anchor { id: Id, local_y: f32, screen_y: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowPlacement<Id> {
    pub id: Id,
    pub y: f32,
    pub height: f32,
}

/// Which side of the window a measurement sits on. A scroll offset stays
/// exact only if heights that appear above the window shift it with them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowSide {
    Above,
    Below,
    /// Not yet known: the offset is recomputed from the heights instead of
    /// being carried across this install.
    Unknown,
}

/// The exact scroll state of a container, in content pixels. A scrollbar is
/// either honest about all three numbers or does not draw at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scroll {
    /// The full content extent.
    pub content: f32,
    /// The visible window.
    pub window: f32,
    /// The content y at the top of the window.
    pub offset: f32,
}

/// Placements are in viewport-local coordinates, and exclude offscreen owners.
/// A container's external size is `constraints`, never its children's extent.
pub struct FlowViewport<Id> {
    pub position: FlowPosition<Id>,
    visible: Vec<FlowPlacement<Id>>,
    heights: HashMap<Id, f32>,
    /// Running total of every measured height at the current layout key.
    total: f32,
    /// Owners whose height was invalidated and still needs measuring.
    missing: HashSet<Id>,
    /// Content-space y of the viewport's top edge, when known.
    content_top: Option<f32>,
    measure_key: Option<(u32, u64)>,
    constraints: Constraints,
    /// Final displacement of the requested anchor due to edge clamping.
    correction: f32,
}

impl<Id: Clone + Eq + Hash> Default for FlowViewport<Id> {
    fn default() -> Self {
        Self {
            position: FlowPosition::FollowTail,
            visible: Vec::new(),
            heights: HashMap::new(),
            total: 0.0,
            missing: HashSet::new(),
            content_top: None,
            measure_key: None,
            constraints: Constraints {
                width: 0.0,
                height: 0.0,
                style_generation: 0,
            },
            correction: 0.0,
        }
    }
}

impl<Id: Clone + Eq + Hash> FlowViewport<Id> {
    pub fn visible(&self) -> &[FlowPlacement<Id>] {
        &self.visible
    }
    pub fn constraints(&self) -> Constraints {
        self.constraints
    }
    pub fn follow_tail(&mut self) {
        self.position = FlowPosition::FollowTail;
    }

    pub fn anchor(&mut self, id: Id, local_y: f32, screen_y: f32) {
        assert!(local_y.is_finite() && screen_y.is_finite());
        self.position = FlowPosition::Anchor {
            id,
            local_y,
            screen_y,
        };
        // A jump places the window wherever the anchor lands; the offset is
        // recomputed from the measured heights, never carried across.
        self.content_top = None;
    }

    pub fn pin_top<S: FlowSource<Id = Id>>(&mut self, source: &S) {
        if let Some(id) = source.first() {
            self.anchor(id, 0.0, 0.0);
        } else {
            self.position = FlowPosition::FollowTail;
        }
    }

    /// Forget measurements after a document reset; keep the stable scroll position.
    pub fn clear_measurements(&mut self) {
        self.heights.clear();
        self.total = 0.0;
        self.missing.clear();
        self.content_top = None;
    }

    /// Adopt one measured height; returns how it changed the total.
    fn set_height(&mut self, id: Id, height: f32) -> f32 {
        let previous = self.heights.insert(id.clone(), height).unwrap_or(0.0);
        self.total += height - previous;
        self.missing.remove(&id);
        height - previous
    }

    /// Forget one height without asking for it again: the owner is gone.
    fn drop_height(&mut self, id: &Id) {
        if let Some(height) = self.heights.remove(id) {
            self.total -= height;
        }
        self.missing.remove(id);
    }

    /// Owners whose height was invalidated and must be measured again before
    /// the totals can be trusted. Draining gives them out; installing one
    /// settles it.
    pub fn take_missing(&mut self, limit: usize) -> Vec<Id> {
        let taken: Vec<Id> = self.missing.iter().take(limit).cloned().collect();
        for id in &taken {
            self.missing.remove(id);
        }
        taken
    }

    /// Install an exact height measured away from the frame. Stale widths and
    /// style generations are rejected: an owner measured for another layout
    /// must never influence this one. Returns whether the height was adopted.
    pub fn install_measurement(
        &mut self,
        id: Id,
        width: f32,
        style_generation: u64,
        height: f32,
        side: FlowSide,
    ) -> bool {
        assert!(height.is_finite() && height >= 0.0);
        if self.measure_key != Some((width.to_bits(), style_generation)) {
            return false;
        }
        let delta = self.set_height(id, height);
        match side {
            // Content that appears above the window pushes it down.
            FlowSide::Above => self.content_top = self.content_top.map(|top| top + delta),
            FlowSide::Below => {}
            FlowSide::Unknown => self.content_top = None,
        }
        true
    }

    /// Exact heights installed or measured at the current layout key.
    pub fn measured_height(&self, id: &Id) -> Option<f32> {
        self.heights.get(id).copied()
    }
    pub fn measured_count(&self) -> usize {
        self.heights.len()
    }

    /// Call when an owner's content changes without changing its stable ID.
    pub fn invalidate(&mut self, id: &Id) {
        self.drop_height(id);
        self.missing.insert(id.clone());
        self.content_top = None;
    }

    /// Transfer an anchor on replacement; old measurements cannot be reused.
    pub fn replace(&mut self, old: &Id, new: Id) {
        self.drop_height(old);
        self.drop_height(&new);
        self.missing.insert(new.clone());
        self.content_top = None;
        if let FlowPosition::Anchor { id, .. } = &mut self.position {
            if id == old {
                *id = new;
            }
        }
    }

    /// Preserve the closest surviving visible owner when an anchor is removed.
    /// If no visible owner survives, resume from the source tail.
    pub fn remove<S: FlowSource<Id = Id>>(&mut self, source: &S, removed: &Id) {
        self.drop_height(removed);
        self.content_top = None;
        if let FlowPosition::Anchor { id, .. } = &self.position {
            if id == removed {
                self.fallback(source);
            }
        }
    }

    fn fallback<S: FlowSource<Id = Id>>(&mut self, source: &S) {
        let screen = match &self.position {
            FlowPosition::Anchor { screen_y, .. } => *screen_y,
            FlowPosition::FollowTail => return,
        };
        if let Some(item) = self
            .visible
            .iter()
            .filter(|p| source.contains(&p.id))
            .min_by(|a, b| (a.y - screen).abs().total_cmp(&(b.y - screen).abs()))
        {
            self.position = FlowPosition::Anchor {
                id: item.id.clone(),
                local_y: 0.0,
                screen_y: item.y,
            };
        } else {
            self.position = FlowPosition::FollowTail;
        }
    }

    fn height<S: FlowSource<Id = Id>>(&mut self, source: &mut S, id: &Id) -> f32 {
        if let Some(height) = self.heights.get(id) {
            return *height;
        }
        let height = source.measure(
            id,
            self.constraints.width,
            self.constraints.style_generation,
        );
        assert!(
            height.is_finite() && height >= 0.0,
            "flow owners require finite nonnegative exact heights"
        );
        self.set_height(id.clone(), height);
        height
    }

    /// Walk only as far as needed to cover the viewport. Traversal must be
    /// consistent and acyclic; no inferred heights or full-document prefix sums.
    pub fn layout<S: FlowSource<Id = Id>>(&mut self, source: &mut S, constraints: Constraints) {
        assert!(constraints.width.is_finite() && constraints.width >= 0.0);
        assert!(constraints.height.is_finite() && constraints.height >= 0.0);
        if self.measure_key != Some((constraints.width.to_bits(), constraints.style_generation)) {
            self.clear_measurements();
            self.measure_key = Some((constraints.width.to_bits(), constraints.style_generation));
        }
        self.constraints = constraints;
        if let FlowPosition::Anchor { id, .. } = &self.position {
            if !source.contains(id) {
                self.fallback(source);
            }
        }
        let start = match &self.position {
            FlowPosition::FollowTail => source.last().map(|id| {
                let height = self.height(source, &id);
                (id, constraints.height - height)
            }),
            FlowPosition::Anchor {
                id,
                local_y,
                screen_y,
            } => Some((id.clone(), screen_y - local_y)),
        };
        let Some((id, y)) = start else {
            self.visible.clear();
            return;
        };
        let mut rows = VecDeque::new();
        let height = self.height(source, &id);
        rows.push_back(FlowPlacement {
            id: id.clone(),
            y,
            height,
        });
        // Fill downward, then upward. At the tail, shift the whole window down
        // to avoid blank bottom space, measuring predecessors only as needed.
        self.extend_down(source, &mut rows);
        self.extend_up(source, &mut rows);
        if source.next(&rows.back().unwrap().id).is_none() {
            let gap = (constraints.height - (rows.back().unwrap().y + rows.back().unwrap().height))
                .max(0.0);
            if gap > 0.0 {
                for row in &mut rows {
                    row.y += gap;
                }
                self.extend_up(source, &mut rows);
            }
        }
        // A blank area above the first owner is never scrollable.
        if source.previous(&rows.front().unwrap().id).is_none() && rows.front().unwrap().y > 0.0 {
            let shift = rows.front().unwrap().y;
            for row in &mut rows {
                row.y -= shift;
            }
            self.extend_down(source, &mut rows);
        }
        self.correction = rows.iter().find(|p| p.id == id).unwrap().y - y;
        // The clamp moved the content on screen; the offset follows it.
        self.content_top = self.content_top.map(|top| top - self.correction);
        self.visible = rows
            .into_iter()
            .filter(|p| p.height > 0.0 && p.y < constraints.height && p.y + p.height > 0.0)
            .collect();
        // Normalize to a visible owner so future layouts never walk from a
        // distant offscreen anchor. The zero-height viewport retains its anchor.
        if constraints.height > 0.0 && !matches!(self.position, FlowPosition::FollowTail) {
            let anchor_visible = match &self.position {
                FlowPosition::Anchor { id, .. } => self.visible.iter().any(|p| &p.id == id),
                FlowPosition::FollowTail => false,
            };
            if anchor_visible {
                if let FlowPosition::Anchor { screen_y, .. } = &mut self.position {
                    *screen_y += self.correction;
                }
            } else if let Some(first) = self.visible.first() {
                self.position = FlowPosition::Anchor {
                    id: first.id.clone(),
                    local_y: 0.0,
                    screen_y: first.y,
                };
            }
        }
    }

    fn extend_down<S: FlowSource<Id = Id>>(
        &mut self,
        source: &mut S,
        rows: &mut VecDeque<FlowPlacement<Id>>,
    ) {
        while rows.back().unwrap().y + rows.back().unwrap().height < self.constraints.height {
            let Some(id) = source.next(&rows.back().unwrap().id) else {
                break;
            };
            let y = rows.back().unwrap().y + rows.back().unwrap().height;
            let height = self.height(source, &id);
            rows.push_back(FlowPlacement { id, y, height });
        }
    }
    fn extend_up<S: FlowSource<Id = Id>>(
        &mut self,
        source: &mut S,
        rows: &mut VecDeque<FlowPlacement<Id>>,
    ) {
        while rows.front().unwrap().y > 0.0 {
            let Some(id) = source.previous(&rows.front().unwrap().id) else {
                break;
            };
            let height = self.height(source, &id);
            let y = rows.front().unwrap().y - height;
            rows.push_front(FlowPlacement { id, y, height });
        }
    }

    /// Positive delta scrolls toward the tail. Returns the unconsumed signed
    /// delta for a parent viewport. Requires an up-to-date layout.
    pub fn wheel<S: FlowSource<Id = Id>>(&mut self, source: &mut S, delta: f32) -> f32 {
        assert!(delta.is_finite());
        if delta == 0.0 || self.constraints.height == 0.0 || self.visible.is_empty() {
            return delta;
        }
        let first = self.visible[0].clone();
        // Scrolling is a content translation: the offset keeps pace with it.
        let content_top = self.content_top.map(|top| top + delta);
        self.anchor(first.id.clone(), 0.0, first.y - delta);
        self.content_top = content_top;
        let constraints = self.constraints;
        self.layout(source, constraints);
        // Requested anchor is retained during layout even if it scrolls out.
        self.correction
    }

    /// Exact scroll metrics, or `None` while any height is still unknown. The
    /// total counts exactly what is measured: a caller draws a thumb only when
    /// it knows the index covers the whole source (and then it is exact).
    pub fn scroll<S: FlowSource<Id = Id>>(&mut self, source: &mut S) -> Option<Scroll> {
        if !self.missing.is_empty() {
            return None;
        }
        let window = self.constraints.height;
        if matches!(self.position, FlowPosition::FollowTail) {
            self.content_top = Some((self.total - window).max(0.0));
        }
        if self.content_top.is_none() {
            // A jump invalidated the running offset. Walking from the first
            // owner is what an honest offset costs; it is cached after.
            let target = self.visible.first()?.id.clone();
            let mut top = 0.0;
            let mut cursor = source.first()?;
            loop {
                if cursor == target {
                    break;
                }
                top += self.heights.get(&cursor).copied()?;
                cursor = source.next(&cursor)?;
            }
            self.content_top = Some(top);
        }
        Some(Scroll {
            content: self.total,
            window,
            offset: self.content_top?.max(0.0),
        })
    }

    /// Move the window so `offset` sits at its top, walking at most `budget`
    /// owners per call. A drag re-calls until it converges; no single call can
    /// cost more than its budget, whatever the document's size.
    pub fn scroll_to<S: FlowSource<Id = Id>>(
        &mut self,
        source: &mut S,
        offset: f32,
        budget: usize,
    ) -> Option<Scroll> {
        let scroll = self.scroll(source)?;
        let target = offset.clamp(0.0, (scroll.content - scroll.window).max(0.0));
        if (target - scroll.offset).abs() < 0.01 {
            return Some(scroll);
        }
        let first = self.visible.first()?;
        // The window top and the first row's top differ whenever that row is
        // clipped: the walk tracks owners, so it starts at the owner's top.
        let mut at = scroll.offset + first.y;
        let mut cursor = first.id.clone();
        let mut steps = 0;
        if target > at {
            while steps < budget {
                let height = self.heights.get(&cursor).copied()?;
                if target < at + height {
                    break; // the target is inside this owner
                }
                at += height;
                cursor = source.next(&cursor)?;
                steps += 1;
            }
        } else {
            while steps < budget {
                if target >= at {
                    break; // the target is inside this owner
                }
                let previous = source.previous(&cursor)?;
                at -= self.heights.get(&previous).copied()?;
                cursor = previous;
                steps += 1;
            }
        }
        // A budget that runs out mid-walk lands on the owner it reached: the
        // anchor stays valid and the next event walks further.
        let inside = self
            .heights
            .get(&cursor)
            .copied()
            .is_some_and(|height| at <= target && target < at + height);
        let (local_y, landed) = if inside {
            (target - at, target)
        } else {
            (0.0, at)
        };
        self.anchor(cursor, local_y, 0.0);
        self.content_top = Some(landed);
        let constraints = self.constraints;
        self.layout(source, constraints);
        self.scroll(source)
    }
}
