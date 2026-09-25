//! Canonical view, live stream projections and bounded decoded-image ownership.
use super::DocumentUpdate;
use misa_proto::sync::{IndexedTree, StreamUpdate, ViewOp};
use misa_proto::view::{Kind, Node, Span, State};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Changes {
    pub ids: BTreeSet<String>,
    pub full: bool,
}

pub(super) struct DocumentStore {
    tree: IndexedTree,
    root: String,
    streams: BTreeMap<String, Node>,
    images: BTreeMap<String, Arc<image::RgbaImage>>,
}

impl DocumentStore {
    pub fn new(view: Node) -> Self {
        let root = view.id.clone();
        Self {
            tree: IndexedTree::new(view),
            root,
            streams: BTreeMap::new(),
            images: BTreeMap::new(),
        }
    }
    pub fn root(&self) -> &str {
        &self.root
    }
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.tree.node(id)
    }
    /// Fields in list items are embedded in their list owner, not separate index entries.
    /// The cache owner of a node embedded inside a list item is its indexed list.
    pub fn cache_owner<'a>(&'a self, id: &'a str) -> &'a str {
        if self.tree.contains(id) {
            return id;
        }
        fn contains(node: &Node, id: &str) -> bool {
            node.id == id
                || node.children.iter().any(|child| contains(child, id))
                || matches!(&node.kind, Kind::List { items, .. } if items.iter().flatten().any(|child| contains(child, id)))
        }
        self.tree
            .nodes()
            .find(|node| contains(node, id))
            .map_or(id, |node| node.id.as_str())
    }
    pub fn field(&self, id: &str, field: &str) -> Option<&misa_proto::view::Field> {
        fn find<'a>(node: &'a Node, id: &str, field: &str) -> Option<&'a misa_proto::view::Field> {
            if node.id == id {
                if let Kind::Fields { fields } = &node.kind {
                    return fields.iter().find(|value| value.id == field);
                }
            }
            if let Kind::List { items, .. } = &node.kind {
                for child in items.iter().flatten() {
                    if let Some(found) = find(child, id, field) {
                        return Some(found);
                    }
                }
            }
            for child in &node.children {
                if let Some(found) = find(child, id, field) {
                    return Some(found);
                }
            }
            None
        }
        self.tree
            .node(id)
            .and_then(|node| find(node, id, field))
            .or_else(|| self.tree.nodes().find_map(|node| find(node, id, field)))
    }
    pub fn stream_or_node(&self, id: &str) -> Option<&Node> {
        self.streams.get(id).or_else(|| self.node(id))
    }
    pub fn contains(&self, id: &str) -> bool {
        self.tree.contains(id)
    }
    pub fn parent(&self, id: &str) -> Option<&str> {
        self.tree.parent(id)
    }
    pub fn children(&self, id: &str) -> Vec<String> {
        self.tree.children(id)
    }
    pub fn subtree(&self, id: &str) -> Option<Node> {
        self.tree.subtree(id)
    }
    #[cfg(test)]
    pub fn snapshot(&self) -> Node {
        self.tree.snapshot()
    }
    #[cfg(test)]
    pub fn streams_empty(&self) -> bool {
        self.streams.is_empty()
    }
    pub fn image_ref(&self, hash: &str) -> Option<&Arc<image::RgbaImage>> {
        self.images.get(hash)
    }
    pub fn has_image(&self, hash: &str) -> bool {
        self.images.contains_key(hash)
    }
    pub fn decoded_image_bytes(&self) -> usize {
        self.images.values().map(|image| image.as_raw().len()).sum()
    }
    pub fn visible_streams(&self) -> Vec<String> {
        self.streams
            .iter()
            .filter(|(id, node)| {
                let owner = id.rsplit_once('.').map(|(owner, _)| owner).unwrap_or(id);
                !self.tree.contains(owner)
                    && matches!(&node.kind, Kind::Text { spans } if spans.iter().any(|span| !span.text.is_empty()))
            })
            .map(|(id, _)| id.clone())
            .collect()
    }
    pub fn stream_parent(&self) -> &str {
        if self.contains("transcript") {
            "transcript"
        } else {
            &self.root
        }
    }
    fn live_hashes(&self) -> BTreeSet<String> {
        let mut hashes = BTreeSet::new();
        for node in self.tree.nodes() {
            image_hashes(node, &mut hashes);
        }
        hashes
    }
    fn prune_images(&mut self, changes: &mut Changes) {
        let live = self.live_hashes();
        let before = self.images.len();
        self.images.retain(|hash, _| live.contains(hash));
        // Display lists retain their own Arcs. Drop them whenever an image is removed.
        changes.full |= self.images.len() != before;
    }
    fn reset(&mut self, view: Node) -> Changes {
        self.root = view.id.clone();
        self.tree = IndexedTree::new(view);
        self.streams.clear();
        let mut changes = Changes {
            full: true,
            ..Changes::default()
        };
        self.prune_images(&mut changes);
        changes
    }
    pub fn observe(&mut self, update: &DocumentUpdate<'_>) -> Result<Changes, String> {
        let mut changes = Changes::default();
        match update {
            DocumentUpdate::Reset { tree, streams } => {
                changes = self.reset((*tree).clone());
                self.reset_streams(streams, &mut changes);
            }
            DocumentUpdate::Changed {
                tree,
                live,
                reset_live,
            } => {
                for op in *tree {
                    match op {
                        ViewOp::Insert { parent, .. } => {
                            changes.ids.insert(parent.clone());
                        }
                        ViewOp::Remove { id } | ViewOp::Replace { id, .. } => {
                            // Capture the old ancestry before removal or replacement.
                            let mut cursor = Some(id.as_str());
                            while let Some(owner) = cursor {
                                changes.ids.insert(owner.to_string());
                                cursor = self.parent(owner);
                            }
                            let mut pending = self.children(id);
                            while let Some(child) = pending.pop() {
                                pending.extend(self.children(&child));
                                changes.ids.insert(child);
                            }
                        }
                    }
                    self.tree.apply(op)?;
                }
                if !tree.is_empty() {
                    self.prune_images(&mut changes);
                }
                if *reset_live {
                    self.reset_streams(&[], &mut changes);
                }
                for update in *live {
                    self.apply_stream(update, &mut changes);
                }
            }
            DocumentUpdate::Notice(_) => {}
        }
        Ok(changes)
    }
    fn invalidate_stream_projection(&self, changes: &mut Changes) {
        changes.ids.insert("streams".into());
        changes.ids.insert(self.stream_parent().into());
    }
    fn reset_streams(&mut self, streams: &[misa_proto::sync::Stream], changes: &mut Changes) {
        changes.ids.extend(self.streams.keys().cloned());
        self.streams.clear();
        for stream in streams {
            self.streams.insert(
                stream.id.clone(),
                stream_node(&stream.id, &stream.role, &stream.text),
            );
            changes.ids.insert(stream.id.clone());
        }
        self.invalidate_stream_projection(changes);
    }
    fn apply_stream(&mut self, update: &StreamUpdate, changes: &mut Changes) {
        match update {
            StreamUpdate::Current { stream } => {
                changes.ids.insert(stream.id.clone());
                self.streams.insert(
                    stream.id.clone(),
                    stream_node(&stream.id, &stream.role, &stream.text),
                );
            }
            StreamUpdate::Append { id, text, .. } => {
                changes.ids.insert(id.clone());
                if let Some(Node {
                    kind: Kind::Text { spans },
                    ..
                }) = self.streams.get_mut(id)
                {
                    if let Some(span) = spans.first_mut() {
                        span.text.push_str(text);
                    }
                }
            }
            StreamUpdate::End { id } => {
                changes.ids.insert(id.clone());
                self.streams.remove(id);
            }
        }
        self.invalidate_stream_projection(changes);
    }
    pub fn image(&mut self, hash: String, image: Arc<image::RgbaImage>) -> ImageChange {
        if !self.live_hashes().contains(&hash) {
            return ImageChange::Ignored;
        }
        const MAX_BYTES: usize = 32 * 1024 * 1024;
        let bytes = image.as_raw().len();
        if bytes > MAX_BYTES {
            return ImageChange::TooLarge;
        }
        self.images.remove(&hash);
        let mut total: usize = self.images.values().map(|image| image.as_raw().len()).sum();
        let mut evicted = false;
        while total.saturating_add(bytes) > MAX_BYTES {
            let Some((_, image)) = self.images.pop_first() else {
                break;
            };
            total -= image.as_raw().len();
            evicted = true;
        }
        self.images.insert(hash.clone(), image);
        let ids = self
            .tree
            .nodes()
            .filter_map(|node| {
                let mut hashes = BTreeSet::new();
                image_hashes(node, &mut hashes);
                hashes.contains(&hash).then(|| node.id.clone())
            })
            .collect();
        ImageChange::Loaded(Changes { ids, full: evicted })
    }
}

pub(super) enum ImageChange {
    Ignored,
    TooLarge,
    Loaded(Changes),
}

fn stream_node(id: &str, role: &str, text: &str) -> Node {
    Node::text(role, [Span::plain(text)])
        .id(id)
        .state(State::Streaming)
}
fn image_hashes(node: &Node, hashes: &mut BTreeSet<String>) {
    if let Kind::Image { blob, .. } = &node.kind {
        hashes.insert(blob.hash.clone());
    }
    for child in &node.children {
        image_hashes(child, hashes);
    }
    if let Kind::List { items, .. } = &node.kind {
        for child in items.iter().flatten() {
            image_hashes(child, hashes);
        }
    }
}
