//! Retained owner display lists and the policy for invalidating them. The document
//! supplies ancestry; only painted, visible moving owners schedule pulse work.
use super::Control;
use super::interaction::GroupGeometry;
use super::{PULSE_PERIOD, document};
use misa_pixel_ui::FieldViewport;
use misa_pixel_ui::Op;
use misa_proto::view::{Kind, Node};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub(super) struct IndicatorBounds {
    pub(super) id: String,
    pub(super) top: f32,
    pub(super) bottom: f32,
}

/// Owner display lists kept beyond the visible frame. Exact heights live in the
/// viewport's measurement index; this cache is a bounded prewarm and may drop
/// any owner at any time without changing what a frame paints.
pub(super) const SCENE_CACHE_OWNERS: usize = 384;

struct Placement {
    id: String,
    y: f32,
}

pub(super) struct Cached {
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) ops: Arc<Vec<Op>>,
    pub(super) geometry: GroupGeometry,
    pub(super) viewport_effects: Vec<(Control, FieldViewport)>,
    children: Vec<Placement>,
    has_moving: bool,
    phase: Option<u64>,
}

#[derive(Default)]
pub(super) struct RetainedScenes {
    cache: BTreeMap<String, Arc<Cached>>,
    used: BTreeMap<String, u64>,
    clock: u64,
    row_keys: BTreeMap<String, BTreeSet<String>>,
    moving: BTreeSet<String>,
    stack: Vec<Vec<Placement>>,
    placed: Vec<IndicatorBounds>,
    viewport_height: f32,
    width: u32,
    phase: u64,
    #[cfg(test)]
    rendered_nodes: usize,
    #[cfg(test)]
    measured_owners: usize,
    #[cfg(test)]
    placed_owners: usize,
}

impl RetainedScenes {
    pub(super) fn clear(&mut self) {
        self.cache.clear();
        self.used.clear();
        self.row_keys.clear();
        self.moving.clear();
        self.stack.clear();
        self.placed.clear();
    }

    fn touch(&mut self, id: &str) {
        self.clock += 1;
        self.used.insert(id.to_owned(), self.clock);
    }

    /// The owner that owns a row or trailing fragment's bookkeeping.
    fn owner_of(key: &str) -> Option<String> {
        if let Some(rest) = key.strip_prefix("\0row:") {
            rest.rsplit_once(':').map(|(owner, _)| owner.to_owned())
        } else {
            key.strip_prefix("\0end:").map(str::to_owned)
        }
    }

    fn evict(&mut self, key: &str) {
        self.cache.remove(key);
        self.used.remove(key);
        self.moving.remove(key);
        if let Some(owner) = Self::owner_of(key)
            && let Some(keys) = self.row_keys.get_mut(&owner)
        {
            keys.remove(key);
        }
    }

    /// Bound prewarm memory: evict the least recently used owner display lists.
    /// Visible owners are touched every frame and cannot be evicted.
    fn trim_to_limit(&mut self) {
        if self.cache.len() <= SCENE_CACHE_OWNERS {
            return;
        }
        let mut order: Vec<(u64, String)> = self
            .cache
            .keys()
            .map(|key| (self.used.get(key).copied().unwrap_or(0), key.clone()))
            .collect();
        order.sort_unstable();
        for (_, key) in order
            .into_iter()
            .take(self.cache.len() - SCENE_CACHE_OWNERS)
        {
            self.evict(&key);
        }
    }

    /// Install one background result's owner display lists. Entries are keyed
    /// exactly as the frame keys them, so a current result never shadows a
    /// differently measured owner.
    pub(super) fn install(&mut self, other: Self) {
        for (owner, keys) in other.row_keys {
            self.row_keys.entry(owner).or_default().extend(keys);
        }
        for id in other.moving {
            self.moving.insert(id);
        }
        for (key, cached) in other.cache {
            self.cache.insert(key.clone(), cached);
            self.touch(&key);
        }
        self.trim_to_limit();
    }

    pub(super) fn clear_owner(&mut self, id: &str) {
        if let Some(keys) = self.row_keys.remove(id) {
            for key in keys {
                self.evict(&key);
            }
        }
        self.evict(id);
    }

    pub(super) fn invalidate_end(&mut self, id: &str) {
        self.evict(&format!("\0end:{id}"));
    }

    pub(super) fn row_keys(&self, id: &str) -> impl Iterator<Item = &String> {
        self.row_keys.get(id).into_iter().flatten()
    }

    pub(super) fn invalidate(&mut self, id: &str, document: &document::DocumentStore) {
        // Live projections are outside the canonical index. A local disclosure
        // toggle must invalidate their synthetic parent as well as the stream.
        if !document.contains(id) && document.stream_or_node(id).is_some() {
            self.cache.remove(id);
            self.cache.remove("streams");
            if document.row_count(document.stream_parent()).is_some() {
                self.invalidate_end(document.stream_parent());
            } else {
                self.invalidate(document.stream_parent(), document);
            }
            return;
        }
        if let Some((owner, row)) = document.fragment_owner(id) {
            self.evict(&format!("\0row:{owner}:{row}"));
            let mut cursor = Some(owner);
            while let Some(id) = cursor {
                self.evict(id);
                cursor = document.parent(id);
            }
        } else {
            let mut cursor = Some(document.cache_owner(id).to_string());
            while let Some(id) = cursor {
                self.clear_owner(&id);
                self.moving.remove(&id);
                cursor = document.parent(&id).map(str::to_owned);
            }
        }
    }

    pub(super) fn invalidate_document(
        &mut self,
        changes: document::Changes,
        document: &document::DocumentStore,
    ) {
        if changes.full {
            self.clear();
        } else {
            for id in changes.ids {
                self.invalidate(&id, document);
            }
            for id in changes.end_ids {
                self.invalidate_end(&id);
            }
        }
    }

    pub(super) fn begin_frame(&mut self, width: u32, elapsed: Duration) {
        self.phase = ((elapsed.as_nanos() / PULSE_PERIOD.as_nanos()) % 4) as u64;
        if self.width != width {
            self.clear();
            self.width = width;
        }
        #[cfg(test)]
        {
            self.rendered_nodes = 0;
            self.measured_owners = 0;
            self.placed_owners = 0;
        }
    }

    pub(super) fn phase(&self) -> u64 {
        self.phase
    }

    pub(super) fn get(&mut self, id: &str, width: f32) -> Option<Arc<Cached>> {
        let cached = self
            .cache
            .get(id)
            .filter(|cached| cached.width == width)
            .cloned();
        if cached.is_some() {
            self.touch(id);
        }
        cached
    }

    pub(super) fn begin_group(&mut self) {
        self.stack.push(Vec::new());
    }

    pub(super) fn begin_placement(&mut self, height: f32) {
        self.placed.clear();
        self.viewport_height = height;
    }

    pub(super) fn observe_status(&mut self, model: &Node) {
        let listed = match &model.kind {
            Kind::List { items, .. } => items.iter().flatten().collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let moving =
            model.children.iter().chain(listed).any(|child| {
                child.role == "indicator.activity" && indicator_value(child) != "ready"
            });
        if moving {
            self.moving.insert(model.id.clone());
        } else {
            self.moving.remove(&model.id);
        }
    }

    pub(super) fn finish_group(
        &mut self,
        id: &str,
        width: f32,
        height: f32,
        ops: Vec<Op>,
        geometry: GroupGeometry,
        status: bool,
        viewport_effects: Vec<(Control, FieldViewport)>,
    ) -> Arc<Cached> {
        let children = self.stack.pop().expect("retained group stack");
        let moving = status && self.moving.contains(id);
        let cached = Arc::new(Cached {
            width,
            height,
            ops: Arc::new(ops),
            geometry,
            viewport_effects,
            has_moving: moving || !children.is_empty(),
            children,
            phase: moving.then_some(self.phase),
        });
        if let Some(rest) = id.strip_prefix("\0row:") {
            if let Some((owner, _)) = rest.rsplit_once(':') {
                self.row_keys
                    .entry(owner.to_owned())
                    .or_default()
                    .insert(id.to_owned());
            }
        } else if let Some(owner) = id.strip_prefix("\0end:") {
            self.row_keys
                .entry(owner.to_owned())
                .or_default()
                .insert(id.to_owned());
        }
        self.cache.insert(id.to_string(), cached.clone());
        self.touch(id);
        self.trim_to_limit();
        cached
    }

    pub(super) fn place(&mut self, id: &str, cached: &Cached, y: f32) {
        if self.stack.is_empty() {
            // Only a placed root makes its descendants visible. Walk the retained
            // owner links, not the document or its unplaced cached groups.
            let children: Vec<(String, f32)> = cached
                .children
                .iter()
                .map(|child| (child.id.clone(), y + child.y))
                .collect();
            for (child, child_y) in children {
                self.collect_placed(&child, child_y);
            }
            if cached.phase.is_some() {
                // Root itself may be the moving owner.
                self.placed.push(IndicatorBounds {
                    id: id.to_string(),
                    top: y,
                    bottom: y + cached.height,
                });
            }
            return;
        }
        if cached.has_moving {
            self.stack
                .last_mut()
                .expect("retained group stack")
                .push(Placement {
                    id: id.to_string(),
                    y,
                });
        }
    }

    /// Children of a placed owner stay as recently used as their parent: an
    /// evicted child would silently freeze its pulse indicator.
    fn collect_placed(&mut self, id: &str, y: f32) {
        self.touch(id);
        let Some(cached) = self.cache.get(id).cloned() else {
            return;
        };
        if cached.phase.is_some() {
            self.placed.push(IndicatorBounds {
                id: id.to_string(),
                top: y,
                bottom: y + cached.height,
            });
        }
        for child in &cached.children {
            let (child, child_y) = (child.id.clone(), y + child.y);
            self.collect_placed(&child, child_y);
        }
    }

    fn visible(&self) -> impl Iterator<Item = &str> {
        self.placed
            .iter()
            .filter(|bounds| {
                bounds.top < self.viewport_height
                    && bounds.bottom > 0.0
                    && self.moving.contains(&bounds.id)
                    && self.cache.contains_key(&bounds.id)
            })
            .map(|bounds| bounds.id.as_str())
    }

    pub(super) fn animating(&self) -> bool {
        self.visible().next().is_some()
    }

    pub(super) fn next_deadline(&self, elapsed: Duration) -> Option<Duration> {
        self.animating()
            .then(|| misa_window_core::next_deadline(elapsed, PULSE_PERIOD))
    }

    /// Only stale visible owners and their ancestors are invalidated. Hidden owners
    /// keep their lists until visible again, and idle frames do not walk history.
    pub(super) fn invalidate_stale_visible(&mut self, document: &document::DocumentStore) -> bool {
        let stale: Vec<_> = self
            .visible()
            .filter(|id| self.cache[*id].phase != Some(self.phase))
            .map(str::to_owned)
            .collect();
        let changed = !stale.is_empty();
        for id in stale {
            self.invalidate(&id, document);
        }
        changed
    }

    #[cfg(test)]
    pub(super) fn node_rendered(&mut self) {
        self.rendered_nodes += 1;
    }
    #[cfg(test)]
    pub(super) fn owner_measured(&mut self) {
        self.measured_owners += 1;
    }
    #[cfg(test)]
    pub(super) fn owner_placed(&mut self) {
        self.placed_owners += 1;
    }
    #[cfg(test)]
    pub(super) fn owner_counts(&self) -> (usize, usize) {
        (self.measured_owners, self.placed_owners)
    }
    #[cfg(test)]
    pub(super) fn rendered_nodes(&self) -> usize {
        self.rendered_nodes
    }
    #[cfg(test)]
    pub(super) fn cached(&self, id: &str) -> &Cached {
        &self.cache[id]
    }
    #[cfg(test)]
    pub(super) fn contains(&self, id: &str) -> bool {
        self.cache.contains_key(id)
    }
    #[cfg(test)]
    pub(super) fn placed_indicators(&self) -> &[IndicatorBounds] {
        &self.placed
    }
}

/// The text of an indicator fact, recursing through wrapper sections.
pub(super) fn indicator_value(node: &Node) -> String {
    match &node.kind {
        Kind::Fact { value } => misa_render::fact::format(&node.role, value),
        Kind::Meter { value, max, .. } => format!(
            "{}/{}",
            misa_render::fact::count(Some(*value as i64), ""),
            misa_render::fact::count(Some(*max as i64), "")
        ),
        Kind::Status { text } => text.clone(),
        Kind::Text { spans } => spans.iter().map(|span| span.text.as_str()).collect(),
        _ => node
            .children
            .iter()
            .map(indicator_value)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    }
}
