//! Canonical view, live stream projections and bounded decoded-image ownership.
use super::DocumentUpdate;
use misa_proto::sync::{IndexedTree, Stream, StreamUpdate, ViewOp};
use misa_proto::view::{Kind, Node, Span, State};
#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Changes {
    pub ids: BTreeSet<String>,
    /// A projected stream changed only this fragment's trailing content.
    pub end_ids: BTreeSet<String>,
    /// Owners whose semantic text may have changed, not incidental ancestors.
    pub retired: BTreeSet<String>,
    pub full: bool,
}

pub(super) struct DocumentStore {
    pub(super) tree: IndexedTree,
    root: String,
    table_columns: BTreeMap<String, usize>,
    // Embedded list descendants are addressed by their indexed owner and row.
    embedded_rows: BTreeMap<String, (String, usize)>,
    embedded_by_owner: BTreeMap<String, BTreeSet<String>>,
    #[cfg(test)]
    indexed_nodes_visited: Cell<usize>,
    #[cfg(test)]
    tables_visited: Cell<usize>,
    streams: BTreeMap<String, LiveStream>,
    // Nonempty streams grouped by owner, including those suppressed by the tree.
    streams_by_owner: BTreeMap<String, BTreeSet<(String, u8, String)>>,
    // Exactly the nonempty streams whose owner is absent from the tree.
    visible_stream_order: BTreeSet<(String, u8, String)>,
    #[cfg(test)]
    visibility_checks: Cell<usize>,
    images: BTreeMap<String, Arc<image::RgbaImage>>,
}

impl DocumentStore {
    pub fn new(view: Node) -> Self {
        let root = view.id.clone();
        let tree = IndexedTree::new(view);
        let table_columns = table_columns(&tree);
        let mut store = Self {
            tree,
            root,
            table_columns,
            embedded_rows: BTreeMap::new(),
            embedded_by_owner: BTreeMap::new(),
            #[cfg(test)]
            indexed_nodes_visited: Cell::new(0),
            #[cfg(test)]
            tables_visited: Cell::new(0),
            streams: BTreeMap::new(),
            streams_by_owner: BTreeMap::new(),
            visible_stream_order: BTreeSet::new(),
            #[cfg(test)]
            visibility_checks: Cell::new(0),
            images: BTreeMap::new(),
        };
        let ids: Vec<_> = store.tree.nodes().map(|node| node.id.clone()).collect();
        for id in ids {
            store.index_owner(&id);
        }
        store
    }
    pub fn root(&self) -> &str {
        &self.root
    }
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.tree.node(id)
    }
    pub(super) fn set_measurement_columns(&mut self, id: &str, columns: usize) {
        self.table_columns.insert(id.to_owned(), columns);
    }
    pub(super) fn table_columns(&self, id: &str) -> usize {
        self.table_columns.get(id).copied().unwrap_or(1)
    }
    pub(super) fn fragment_owner(&self, id: &str) -> Option<(&str, usize)> {
        self.embedded_rows
            .get(id)
            .map(|(owner, row)| (owner.as_str(), *row))
    }
    fn unindex_owner(&mut self, id: &str) {
        if let Some(ids) = self.embedded_by_owner.remove(id) {
            for child in ids {
                self.embedded_rows.remove(&child);
            }
        }
        self.table_columns.remove(id);
    }
    fn index_owner(&mut self, id: &str) {
        let Some(node) = self.tree.node(id) else {
            return;
        };
        #[cfg(test)]
        self.indexed_nodes_visited
            .set(self.indexed_nodes_visited.get() + 1);
        if let Kind::Table { head, rows, .. } = &node.kind {
            #[cfg(test)]
            self.tables_visited.set(self.tables_visited.get() + 1);
            self.table_columns
                .insert(id.to_owned(), table_column_count(head, rows));
        }
        if let Kind::List { items, .. } = &node.kind {
            let mut ids = BTreeSet::new();
            for (row, item) in items.iter().enumerate() {
                for child in item {
                    collect_embedded(child, &mut |id| {
                        ids.insert(id.to_owned());
                        self.embedded_rows
                            .insert(id.to_owned(), (node.id.clone(), row));
                    });
                }
            }
            self.embedded_by_owner.insert(id.to_owned(), ids);
        }
    }
    #[cfg(test)]
    pub(super) fn index_visits(&self) -> (usize, usize) {
        (self.indexed_nodes_visited.get(), self.tables_visited.get())
    }
    /// Fields in list items are embedded in their list owner, not separate index entries.
    /// The cache owner of a node embedded inside a list item is its indexed list.
    pub fn cache_owner<'a>(&'a self, id: &'a str) -> &'a str {
        if self.tree.contains(id) || id == "streams" || self.streams.contains_key(id) {
            return id;
        }
        self.fragment_owner(id).map_or(id, |(owner, _)| owner)
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
            .or_else(|| {
                let (owner, row) = self.fragment_owner(id)?;
                let Kind::List { items, .. } = &self.tree.node(owner)?.kind else {
                    return None;
                };
                items
                    .get(row)?
                    .iter()
                    .find_map(|node| find(node, id, field))
            })
    }
    pub fn stream_or_node(&self, id: &str) -> Option<&Node> {
        self.streams
            .get(id)
            .map(|live| &live.node)
            .or_else(|| self.node(id))
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
    /// Install already-projected, owner-scoped resources in a private measurement store.
    /// No live stream source text or markdown parser state is copied.
    pub(super) fn install_measurement_resources(
        &mut self,
        streams: Vec<Node>,
        images: BTreeMap<String, Arc<image::RgbaImage>>,
    ) {
        self.images = images;
        for node in streams {
            let id = node.id.clone();
            self.streams.insert(
                id.clone(),
                LiveStream {
                    role: node.role.clone(),
                    text: String::new(),
                    parsed: None,
                    node,
                },
            );
            self.insert_stream_key(&id);
        }
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
    fn owner_visible(&self, owner: &str) -> bool {
        #[cfg(test)]
        self.visibility_checks.set(self.visibility_checks.get() + 1);
        !self.tree.contains(owner)
    }
    #[cfg(test)]
    pub(super) fn visibility_checks(&self) -> usize {
        self.visibility_checks.get()
    }
    // Reconcile only streams owned by the changed node, not all suppressed streams.
    fn refresh_stream_owner(&mut self, owner: &str) {
        if let Some(keys) = self.streams_by_owner.get(owner) {
            let visible = self.owner_visible(owner);
            for key in keys {
                if visible {
                    self.visible_stream_order.insert(key.clone());
                } else {
                    self.visible_stream_order.remove(key);
                }
            }
        }
    }
    fn refresh_stream_subtree(&mut self, id: &str) {
        let mut pending = vec![id.to_owned()];
        while let Some(owner) = pending.pop() {
            pending.extend(self.children(&owner));
            self.refresh_stream_owner(&owner);
        }
    }
    fn remove_stream_key(&mut self, id: &str) {
        let key = stream_order(id);
        self.visible_stream_order.remove(&key);
        if let Some(keys) = self.streams_by_owner.get_mut(&key.0) {
            keys.remove(&key);
            if keys.is_empty() {
                self.streams_by_owner.remove(&key.0);
            }
        }
    }
    fn insert_stream_key(&mut self, id: &str) {
        let key = stream_order(id);
        if self.owner_visible(&key.0) {
            self.visible_stream_order.insert(key.clone());
        }
        self.streams_by_owner
            .entry(key.0.clone())
            .or_default()
            .insert(key);
    }
    pub fn visible_streams(&self) -> Vec<String> {
        self.visible_stream_order
            .iter()
            .map(|(_, _, id)| id.clone())
            .collect()
    }
    pub(super) fn first_stream(&self) -> Option<String> {
        self.visible_stream_order
            .first()
            .map(|(_, _, id)| id.clone())
    }
    pub(super) fn last_stream(&self) -> Option<String> {
        self.visible_stream_order
            .last()
            .map(|(_, _, id)| id.clone())
    }
    pub(super) fn next_stream(&self, id: &str) -> Option<String> {
        use std::ops::Bound::{Excluded, Unbounded};
        self.visible_stream_order
            .range((Excluded(stream_order(id)), Unbounded))
            .next()
            .map(|(_, _, id)| id.clone())
    }
    pub(super) fn previous_stream(&self, id: &str) -> Option<String> {
        use std::ops::Bound::{Excluded, Unbounded};
        self.visible_stream_order
            .range((Unbounded, Excluded(stream_order(id))))
            .next_back()
            .map(|(_, _, id)| id.clone())
    }
    pub(super) fn has_stream(&self, id: &str) -> bool {
        self.visible_stream_order.contains(&stream_order(id))
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
        self.table_columns.clear();
        self.embedded_rows.clear();
        self.embedded_by_owner.clear();
        let ids: Vec<_> = self.tree.nodes().map(|node| node.id.clone()).collect();
        for id in ids {
            self.index_owner(&id);
        }
        self.streams.clear();
        self.streams_by_owner.clear();
        self.visible_stream_order.clear();
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
                    let mut removed_owners = Vec::new();
                    match op {
                        ViewOp::Insert { parent, .. } => {
                            changes.ids.insert(parent.clone());
                            changes.retired.insert(parent.clone());
                        }
                        ViewOp::Remove { id } | ViewOp::Replace { id, .. } => {
                            removed_owners.push(id.clone());
                            changes.retired.insert(id.clone());
                            changes.retired.extend(self.children(id));
                            // Capture the old ancestry before removal or replacement.
                            let mut cursor = Some(id.as_str());
                            while let Some(owner) = cursor {
                                changes.ids.insert(owner.to_string());
                                // Registered composites paint their indexed descendants as
                                // one selection owner; changing a child retires that text.
                                if self.node(owner).is_some_and(|node| {
                                    matches!(
                                        node.role.as_str(),
                                        "queue" | "status.indicators" | "message.group.footer"
                                    )
                                }) {
                                    changes.retired.insert(owner.to_string());
                                }
                                cursor = self.parent(owner);
                            }
                            let mut pending = self.children(id);
                            while let Some(child) = pending.pop() {
                                pending.extend(self.children(&child));
                                changes.retired.insert(child.clone());
                                changes.ids.insert(child.clone());
                                removed_owners.push(child);
                            }
                        }
                    }
                    // Only the old and new indexed subtrees can change row addresses
                    // or table widths. No walk of unrelated history is needed.
                    if let ViewOp::Remove { id } | ViewOp::Replace { id, .. } = op {
                        for owner in
                            std::iter::once(id.clone()).chain(removed_owners.iter().cloned())
                        {
                            self.unindex_owner(&owner);
                        }
                    }
                    self.tree.apply(op)?;
                    if let ViewOp::Insert { node, .. } | ViewOp::Replace { node, .. } = op {
                        let mut pending = vec![node.id.clone()];
                        while let Some(id) = pending.pop() {
                            pending.extend(self.children(&id));
                            self.index_owner(&id);
                        }
                    }
                    for owner in removed_owners {
                        self.refresh_stream_owner(&owner);
                    }
                    if let ViewOp::Insert { node, .. } | ViewOp::Replace { node, .. } = op {
                        // The inserted/replaced subtree can itself contain stream owners.
                        self.refresh_stream_subtree(&node.id);
                    }
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
        if self.row_count(self.stream_parent()).is_some() {
            changes.end_ids.insert(self.stream_parent().into());
        } else {
            changes.ids.insert(self.stream_parent().into());
        }
    }
    fn reset_streams(&mut self, streams: &[misa_proto::sync::Stream], changes: &mut Changes) {
        changes.ids.extend(self.streams.keys().cloned());
        changes.retired.extend(self.streams.keys().cloned());
        self.streams.clear();
        self.streams_by_owner.clear();
        self.visible_stream_order.clear();
        for stream in streams {
            self.streams
                .insert(stream.id.clone(), LiveStream::new(stream));
            if !stream.text.is_empty() {
                self.insert_stream_key(&stream.id);
            }
            changes.ids.insert(stream.id.clone());
        }
        self.invalidate_stream_projection(changes);
    }
    fn apply_stream(&mut self, update: &StreamUpdate, changes: &mut Changes) {
        let id = match update {
            StreamUpdate::Current { stream } => stream.id.as_str(),
            StreamUpdate::Append { id, .. } | StreamUpdate::End { id } => id.as_str(),
        };
        self.remove_stream_key(id);
        changes.retired.insert(id.to_owned());
        match update {
            StreamUpdate::Current { stream } => {
                changes.ids.insert(stream.id.clone());
                self.streams
                    .insert(stream.id.clone(), LiveStream::new(stream));
            }
            StreamUpdate::Append { id, text, .. } => {
                changes.ids.insert(id.clone());
                if let Some(live) = self.streams.get_mut(id) {
                    live.append(text);
                }
            }
            StreamUpdate::End { id } => {
                changes.ids.insert(id.clone());
                self.streams.remove(id);
            }
        }
        if self
            .streams
            .get(id)
            .is_some_and(|live| !live.text.is_empty())
        {
            self.insert_stream_key(id);
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
        ImageChange::Loaded(Changes {
            ids,
            full: evicted,
            ..Changes::default()
        })
    }
}

pub(super) fn table_column_count(head: &[Vec<Span>], rows: &[Vec<Vec<Span>>]) -> usize {
    head.len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(1))
        .max(1)
}

fn table_columns(tree: &IndexedTree) -> BTreeMap<String, usize> {
    tree.nodes()
        .filter_map(|node| {
            if let Kind::Table { head, rows, .. } = &node.kind {
                Some((node.id.clone(), table_column_count(head, rows)))
            } else {
                None
            }
        })
        .collect()
}

fn collect_embedded(node: &Node, visit: &mut impl FnMut(&str)) {
    visit(&node.id);
    for child in &node.children {
        collect_embedded(child, visit);
    }
    if let Kind::List { items, .. } = &node.kind {
        for child in items.iter().flatten() {
            collect_embedded(child, visit);
        }
    }
}

pub(super) enum ImageChange {
    Ignored,
    TooLarge,
    Loaded(Changes),
}

/// The parsed in-flight body is owned by the document, not by the paint pass.
struct LiveStream {
    role: String,
    text: String,
    parsed: Option<misa_markdown::Document>,
    node: Node,
}

impl LiveStream {
    fn new(stream: &Stream) -> Self {
        let mut live = Self {
            role: stream.role.clone(),
            text: String::new(),
            parsed: None,
            node: Node::section(&stream.role)
                .id(&stream.id)
                .state(State::Streaming),
        };
        live.append(&stream.text);
        live
    }

    fn append(&mut self, delta: &str) {
        self.text.push_str(delta);
        let id = self.node.id.clone();
        if thinking_stream(&self.role) {
            // Reasoning is plain text in the settled tree too. Keep the same
            // disclosure identity across appends and let the reader open it.
            self.node = Node::new(
                &self.role,
                Kind::Collapsible {
                    summary: thinking_summary(&self.text),
                },
            )
            .id(&id)
            .state(State::Streaming)
            .child(
                Node::text(format!("{}.text", self.role), [Span::plain(&self.text)])
                    .id(format!("{id}.body")),
            );
            self.parsed = None;
        } else {
            let parsed = misa_markdown::document(&self.role, &self.text, self.parsed.as_ref());
            let mut blocks = parsed.blocks.clone();
            // Positional IDs keep the unchanged prefix's identity across appends.
            // Nested block addresses are assigned by the protocol's address policy.
            for (index, block) in blocks.iter_mut().enumerate() {
                block.id = format!("{id}.block.{index}");
                misa_proto::sync::address(block);
            }
            self.node = Node::section(&self.role)
                .id(&id)
                .state(State::Streaming)
                .children(blocks);
            self.parsed = Some(parsed);
        }
    }
}

pub(super) fn thinking_summary(text: &str) -> Vec<Span> {
    let mut summary = vec![Span::strong("thinking")];
    if !text.is_empty() {
        // A collapsed live disclosure follows the last three lines, like TUI
        // Live; opening it exposes the full, unparsed reasoning body.
        let tail = text.lines().rev().take(3).collect::<Vec<_>>();
        summary.push(Span::plain(format!(
            " · {}",
            tail.into_iter().rev().collect::<Vec<_>>().join(" ↵ ")
        )));
    }
    summary
}

pub(super) fn thinking_stream(role: &str) -> bool {
    role.starts_with("message.assistant.thinking") || role.starts_with("message.thinking")
}

pub(super) fn stream_order(id: &str) -> (String, u8, String) {
    let (owner, suffix) = id.rsplit_once('.').unwrap_or((id, ""));
    let rank = match suffix {
        "thinking" => 0,
        "text" => 1,
        _ => 2,
    };
    (owner.to_string(), rank, id.to_string())
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
