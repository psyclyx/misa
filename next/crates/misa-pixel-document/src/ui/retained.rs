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
        self.row_keys.clear();
        self.moving.clear();
        self.stack.clear();
        self.placed.clear();
    }

    pub(super) fn clear_owner(&mut self, id: &str) {
        if let Some(keys) = self.row_keys.remove(id) {
            for key in keys {
                self.cache.remove(&key);
            }
        }
        self.cache.remove(id);
    }

    pub(super) fn invalidate_end(&mut self, id: &str) {
        let key = format!("\0end:{id}");
        self.cache.remove(&key);
        if let Some(keys) = self.row_keys.get_mut(id) {
            keys.remove(&key);
        }
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
            let key = format!("\0row:{owner}:{row}");
            self.cache.remove(&key);
            if let Some(keys) = self.row_keys.get_mut(owner) {
                keys.remove(&key);
            }
            let mut cursor = Some(owner);
            while let Some(id) = cursor {
                self.cache.remove(id);
                self.moving.remove(id);
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

    pub(super) fn get(&self, id: &str, width: f32) -> Option<Arc<Cached>> {
        self.cache
            .get(id)
            .filter(|cached| cached.width == width)
            .cloned()
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
        cached
    }

    pub(super) fn place(&mut self, id: &str, cached: &Cached, y: f32) {
        if let Some(parent) = self.stack.last_mut() {
            if cached.has_moving {
                parent.push(Placement {
                    id: id.to_string(),
                    y,
                });
            }
        } else {
            // Only a placed root makes its descendants visible. Walk the retained
            // owner links, not the document or its unplaced cached groups.
            for child in &cached.children {
                Self::collect_placed(&self.cache, &child.id, y + child.y, &mut self.placed);
            }
            if cached.phase.is_some() {
                // Root itself may be the moving owner.
                self.placed.push(IndicatorBounds {
                    id: id.to_string(),
                    top: y,
                    bottom: y + cached.height,
                });
            }
        }
    }

    fn collect_placed(
        cache: &BTreeMap<String, Arc<Cached>>,
        id: &str,
        y: f32,
        placed: &mut Vec<IndicatorBounds>,
    ) {
        let Some(cached) = cache.get(id) else {
            return;
        };
        if cached.phase.is_some() {
            placed.push(IndicatorBounds {
                id: id.to_string(),
                top: y,
                bottom: y + cached.height,
            });
        }
        for child in &cached.children {
            Self::collect_placed(cache, &child.id, y + child.y, placed);
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
