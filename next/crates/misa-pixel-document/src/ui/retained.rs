//! Retained owner display lists and the policy for invalidating them. The document
//! supplies ancestry; only painted, visible moving owners schedule pulse work.
use super::interaction::GroupGeometry;
use super::{PULSE_PERIOD, document};
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

pub(super) struct Cached {
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) ops: Arc<Vec<Op>>,
    pub(super) geometry: GroupGeometry,
    indicators: Vec<IndicatorBounds>,
    phase: Option<u64>,
}

#[derive(Default)]
pub(super) struct RetainedScenes {
    cache: BTreeMap<String, Arc<Cached>>,
    moving: BTreeSet<String>,
    stack: Vec<Vec<IndicatorBounds>>,
    width: u32,
    phase: u64,
    #[cfg(test)]
    rendered_nodes: usize,
}

impl RetainedScenes {
    pub(super) fn clear(&mut self) {
        self.cache.clear();
        self.moving.clear();
        self.stack.clear();
    }

    pub(super) fn invalidate(&mut self, id: &str, document: &document::DocumentStore) {
        let mut cursor = Some(document.cache_owner(id).to_string());
        while let Some(id) = cursor {
            self.cache.remove(&id);
            self.moving.remove(&id);
            cursor = document.parent(&id).map(str::to_owned);
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

    pub(super) fn observe_status(&mut self, model: &Node) {
        let moving = model
            .children
            .iter()
            .any(|child| child.role == "indicator.activity" && indicator_value(child) != "ready");
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
    ) -> Arc<Cached> {
        let mut indicators = self.stack.pop().expect("retained group stack");
        let moving = status && self.moving.contains(id);
        if moving {
            indicators.push(IndicatorBounds {
                id: id.to_string(),
                top: 0.0,
                bottom: height,
            });
        }
        let cached = Arc::new(Cached {
            width,
            height,
            ops: Arc::new(ops),
            geometry,
            indicators,
            phase: moving.then_some(self.phase),
        });
        self.cache.insert(id.to_string(), cached.clone());
        cached
    }

    pub(super) fn place(&mut self, cached: &Cached, y: f32) {
        if let Some(parent) = self.stack.last_mut() {
            parent.extend(cached.indicators.iter().map(|bounds| IndicatorBounds {
                id: bounds.id.clone(),
                top: bounds.top + y,
                bottom: bounds.bottom + y,
            }));
        }
    }

    fn visible<'a>(
        &'a self,
        root: &str,
        scroll: f32,
        viewport: f32,
    ) -> impl Iterator<Item = &'a str> {
        let top = 20.0 - scroll;
        self.cache
            .get(root)
            .into_iter()
            .flat_map(|cached| cached.indicators.iter())
            .filter(move |bounds| {
                top + bounds.top < viewport
                    && top + bounds.bottom > 0.0
                    && self.moving.contains(&bounds.id)
                    && self.cache.contains_key(&bounds.id)
            })
            .map(|bounds| bounds.id.as_str())
    }

    pub(super) fn animating(&self, root: &str, scroll: f32, viewport: f32) -> bool {
        self.visible(root, scroll, viewport).next().is_some()
    }

    pub(super) fn next_deadline(
        &self,
        root: &str,
        scroll: f32,
        viewport: f32,
        elapsed: Duration,
    ) -> Option<Duration> {
        self.animating(root, scroll, viewport)
            .then(|| misa_window_core::next_deadline(elapsed, PULSE_PERIOD))
    }

    /// Only stale visible owners and their ancestors are invalidated. Hidden owners
    /// keep their lists until visible again, and idle frames do not walk history.
    pub(super) fn invalidate_stale_visible(
        &mut self,
        document: &document::DocumentStore,
        scroll: f32,
        viewport: f32,
    ) -> bool {
        let stale: Vec<_> = self
            .visible(document.root(), scroll, viewport)
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
    pub(super) fn indicators(&self, id: &str) -> &[IndicatorBounds] {
        &self.cache[id].indicators
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
