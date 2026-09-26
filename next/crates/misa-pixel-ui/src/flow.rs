//! Exact, sparse vertical flow. The source owns ordering and measurements; the
//! viewport owns only a scroll anchor, visible placements, and measured heights.
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// IDs remain stable across edits. `previous`/`next` are indexed navigation,
/// not iterators over a materialized document. `measure` returns the exact height
/// at this width and style generation (including any nested bounded viewport).
pub trait FlowSource {
    type Id: Clone + Eq + Hash;
    fn first(&self) -> Option<Self::Id>;
    fn last(&self) -> Option<Self::Id>;
    fn previous(&self, id: &Self::Id) -> Option<Self::Id>;
    fn next(&self, id: &Self::Id) -> Option<Self::Id>;
    fn contains(&self, id: &Self::Id) -> bool;
    fn measure(&self, id: &Self::Id, width: f32, style_generation: u64) -> f32;
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

/// Placements are in viewport-local coordinates, and exclude offscreen owners.
/// A container's external size is `constraints`, never its children's extent.
pub struct FlowViewport<Id> {
    pub position: FlowPosition<Id>,
    visible: Vec<FlowPlacement<Id>>,
    heights: HashMap<Id, f32>,
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
    }

    pub fn pin_top<S: FlowSource<Id = Id>>(&mut self, source: &S) {
        if let Some(id) = source.first() {
            self.anchor(id, 0.0, 0.0);
        } else {
            self.position = FlowPosition::FollowTail;
        }
    }

    /// Call when an owner's content changes without changing its stable ID.
    pub fn invalidate(&mut self, id: &Id) {
        self.heights.remove(id);
    }

    /// Transfer an anchor on replacement; old measurements cannot be reused.
    pub fn replace(&mut self, old: &Id, new: Id) {
        self.heights.remove(old);
        self.heights.remove(&new);
        if let FlowPosition::Anchor { id, .. } = &mut self.position {
            if id == old {
                *id = new;
            }
        }
    }

    /// Preserve the closest surviving visible owner when an anchor is removed.
    /// If no visible owner survives, resume from the source tail.
    pub fn remove<S: FlowSource<Id = Id>>(&mut self, source: &S, removed: &Id) {
        self.heights.remove(removed);
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

    fn height<S: FlowSource<Id = Id>>(&mut self, source: &S, id: &Id) -> f32 {
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
        self.heights.insert(id.clone(), height);
        height
    }

    /// Walk only as far as needed to cover the viewport. Traversal must be
    /// consistent and acyclic; no inferred heights or full-document prefix sums.
    pub fn layout<S: FlowSource<Id = Id>>(&mut self, source: &S, constraints: Constraints) {
        assert!(constraints.width.is_finite() && constraints.width >= 0.0);
        assert!(constraints.height.is_finite() && constraints.height >= 0.0);
        if self.measure_key != Some((constraints.width.to_bits(), constraints.style_generation)) {
            self.heights.clear();
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
        source: &S,
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
        source: &S,
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
    pub fn wheel<S: FlowSource<Id = Id>>(&mut self, source: &S, delta: f32) -> f32 {
        assert!(delta.is_finite());
        if delta == 0.0 || self.constraints.height == 0.0 || self.visible.is_empty() {
            return delta;
        }
        let first = self.visible[0].clone();
        self.anchor(first.id.clone(), 0.0, first.y - delta);
        let constraints = self.constraints;
        self.layout(source, constraints);
        // Requested anchor is retained during layout even if it scrolls out.
        self.correction
    }
}
