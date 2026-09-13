//! Canonical view revisions and id-addressed, self-contained changes.
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::view::Node;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub epoch: String,
    pub rev: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ViewOp {
    Insert { parent: String, before: Option<String>, node: Node },
    Remove { id: String },
    Replace { id: String, node: Node },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub from: Version,
    pub version: Version,
    pub ops: Vec<ViewOp>,
}

/// In-flight text is separate from the canonical document. Offset is measured in UTF-8 bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stream {
    pub id: String,
    pub role: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "update", rename_all = "snake_case")]
pub enum StreamUpdate {
    Current { stream: Stream },
    Append { id: String, offset: usize, text: String },
    End { id: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ViewSync {
    Snapshot { version: Version, view: Node, streams: Vec<Stream> },
    Changes { version: Version, changes: Vec<Change>, streams: Vec<Stream> },
}

/// An indexed tree with linked siblings: appending and unlinking do not scan a transcript.
/// Recursive `Node` is materialized only when a snapshot or renderer asks for it.
#[derive(Clone, Debug)]
pub struct IndexedTree {
    root: String,
    nodes: HashMap<String, Entry>,
}

#[derive(Clone, Debug)]
struct Entry {
    node: Node,
    parent: Option<String>,
    previous: Option<String>,
    next: Option<String>,
    first: Option<String>,
    last: Option<String>,
}

impl IndexedTree {
    pub fn new(mut root: Node) -> Self {
        address(&mut root);
        let mut tree = Self { root: root.id.clone(), nodes: HashMap::new() };
        tree.store(root, None, None, None);
        tree
    }

    fn store(&mut self, mut node: Node, parent: Option<String>, previous: Option<String>, next: Option<String>) {
        let children = std::mem::take(&mut node.children);
        let id = node.id.clone();
        self.nodes.insert(id.clone(), Entry {
            node, parent, previous, next,
            first: children.first().map(|child| child.id.clone()),
            last: children.last().map(|child| child.id.clone()),
        });
        let mut previous = None;
        let mut children = children.into_iter().peekable();
        while let Some(child) = children.next() {
            let next = children.peek().map(|child| child.id.clone());
            let child_id = child.id.clone();
            self.store(child, Some(id.clone()), previous, next);
            previous = Some(child_id);
        }
    }

    pub fn contains(&self, id: &str) -> bool { self.nodes.contains_key(id) }
    pub fn len(&self) -> usize { self.nodes.len() }
    pub fn is_empty(&self) -> bool { self.nodes.is_empty() }
    pub fn node(&self, id: &str) -> Option<&Node> { self.nodes.get(id).map(|entry| &entry.node) }
    pub fn parent(&self, id: &str) -> Option<&str> { self.nodes.get(id)?.parent.as_deref() }
    pub fn children(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor = self.nodes.get(id).and_then(|entry| entry.first.as_deref());
        while let Some(id) = cursor {
            out.push(id.to_owned());
            cursor = self.nodes.get(id).and_then(|entry| entry.next.as_deref());
        }
        out
    }
    pub fn snapshot(&self) -> Node { self.subtree(&self.root).expect("indexed root exists") }
    pub fn subtree(&self, id: &str) -> Option<Node> {
        let mut node = self.nodes.get(id)?.node.clone();
        node.children = self.children(id).iter().filter_map(|child| self.subtree(child)).collect();
        Some(node)
    }

    fn erase(&mut self, id: &str) {
        for child in self.children(id) { self.erase(&child); }
        self.nodes.remove(id);
    }

    fn check_subtree(&self, node: &Node, replacing: Option<&str>) -> Result<(), String> {
        crate::view::validate(node).map_err(|fault| fault.to_string())?;
        let mut allowed = std::collections::HashSet::new();
        let mut pending = replacing.into_iter().map(str::to_owned).collect::<Vec<_>>();
        while let Some(id) = pending.pop() {
            pending.extend(self.children(&id));
            allowed.insert(id);
        }
        let mut pending = vec![node];
        while let Some(node) = pending.pop() {
            if node.id.is_empty() { return Err("operation node has no identity".into()); }
            if self.contains(&node.id) && !allowed.contains(&node.id) {
                return Err(format!("duplicate node {}", node.id));
            }
            pending.extend(&node.children);
        }
        Ok(())
    }

    /// Apply one op. A receiver discards its accumulator and resynchronizes on an invalid op.
    pub fn apply(&mut self, op: &ViewOp) -> Result<(), String> {
        match op {
            ViewOp::Insert { parent, before, node } => {
                self.check_subtree(node, None)?;
                if self.contains(&node.id) { return Err(format!("duplicate node {}", node.id)); }
                let owner = self.nodes.get(parent).ok_or_else(|| format!("missing parent {parent}"))?;
                let previous = if let Some(before) = before {
                    let next = self.nodes.get(before).ok_or_else(|| format!("missing sibling {before}"))?;
                    if next.parent.as_ref() != Some(parent) { return Err("sibling belongs to another parent".into()); }
                    next.previous.clone()
                } else { owner.last.clone() };
                if let Some(previous) = &previous { self.nodes.get_mut(previous).unwrap().next = Some(node.id.clone()); }
                else { self.nodes.get_mut(parent).unwrap().first = Some(node.id.clone()); }
                if let Some(next) = before { self.nodes.get_mut(next).unwrap().previous = Some(node.id.clone()); }
                else { self.nodes.get_mut(parent).unwrap().last = Some(node.id.clone()); }
                self.store(node.clone(), Some(parent.clone()), previous, before.clone());
            }
            ViewOp::Remove { id } => {
                let entry = self.nodes.get(id).ok_or_else(|| format!("missing node {id}"))?;
                let parent = entry.parent.clone().ok_or("cannot remove the root")?;
                let previous = entry.previous.clone();
                let next = entry.next.clone();
                if let Some(previous) = &previous { self.nodes.get_mut(previous).unwrap().next = next.clone(); }
                else { self.nodes.get_mut(&parent).unwrap().first = next.clone(); }
                if let Some(next) = &next { self.nodes.get_mut(next).unwrap().previous = previous.clone(); }
                else { self.nodes.get_mut(&parent).unwrap().last = previous.clone(); }
                self.erase(id);
            }
            ViewOp::Replace { id, node } => {
                self.check_subtree(node, Some(id))?;
                if node.id != *id { return Err("replacement changes identity".into()); }
                let entry = self.nodes.get(id).ok_or_else(|| format!("missing node {id}"))?;
                let (parent, previous, next) = (entry.parent.clone(), entry.previous.clone(), entry.next.clone());
                self.erase(id);
                self.store(node.clone(), parent, previous, next);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_siblings_keep_order_after_insert_remove_and_replace() {
        let mut tree = IndexedTree::new(Node::section("root").child(Node::section("a").id("a")).child(Node::section("c").id("c")));
        tree.apply(&ViewOp::Insert { parent: "root".into(), before: Some("c".into()), node: Node::section("b").id("b") }).unwrap();
        tree.apply(&ViewOp::Remove { id: "a".into() }).unwrap();
        tree.apply(&ViewOp::Replace { id: "b".into(), node: Node::section("b").id("b").child(Node::section("inside").id("inside")) }).unwrap();
        assert_eq!(tree.children("root"), vec!["b", "c"]);
        assert_eq!(tree.snapshot().children[0].children[0].id, "inside");
        assert_eq!(tree.len(), 4);
    }
}

/// Give immutable content nodes addresses under their nearest semantic owner.
/// List members whose position can move must already carry their domain identity.
pub fn address(node: &mut Node) {
    if node.id.is_empty() { node.id = node.role.clone(); }
    let mut roles = std::collections::BTreeMap::<String, usize>::new();
    for child in &mut node.children {
        let occurrence = roles.entry(child.role.clone()).or_default();
        if child.id.is_empty() { child.id = format!("{}.{}.{}", node.id, child.role, occurrence); }
        *occurrence += 1;
        address(child);
    }
}

/// The client accumulator. Persistent state is exactly `(version, canonical tree)`;
/// streams are an ephemeral overlay and are never written into that tree.
#[derive(Clone, Debug, Default)]
pub struct ClientView {
    version: Option<Version>,
    tree: Option<IndexedTree>,
    streams: std::collections::BTreeMap<String, Stream>,
}

impl ClientView {
    pub fn restore(version: Version, mut tree: Node) -> Result<Self, String> {
        crate::view::validate(&tree).map_err(|fault| fault.to_string())?;
        address(&mut tree);
        crate::view::validate(&tree).map_err(|fault| fault.to_string())?;
        Ok(Self { version: Some(version), tree: Some(IndexedTree::new(tree)), streams: Default::default() })
    }
    pub fn version(&self) -> Option<&Version> { self.version.as_ref() }
    pub fn canonical(&self) -> Option<Node> { self.tree.as_ref().map(IndexedTree::snapshot) }
    pub fn persisted(&self) -> Option<(Version, Node)> { Some((self.version.clone()?, self.canonical()?)) }
    pub fn streams(&self) -> Vec<Stream> { self.streams.values().cloned().collect() }
    pub fn rendered(&self) -> Option<Node> {
        let tree = self.tree.as_ref()?;
        let mut view = tree.snapshot();
        let mut overlay = Node::section("streams").id("streams");
        for stream in self.streams.values() {
            let owner = stream.id.rsplit_once('.').map(|(owner, _)| owner).unwrap_or(&stream.id);
            if stream.text.is_empty() || tree.contains(owner) { continue; }
            overlay.children.push(Node::text(&stream.role, [crate::view::Span::plain(&stream.text)])
                .id(&stream.id).state(crate::view::State::Streaming));
        }
        if !overlay.children.is_empty() {
            if let Some(transcript) = view.children.iter_mut().find(|node| node.id == "transcript") {
                transcript.children.push(overlay);
            } else { view.children.push(overlay); }
        }
        Some(view)
    }

    pub fn receive(&mut self, message: &crate::SessionMsg) -> Result<bool, String> {
        use crate::{SessionEvent, SessionMsg};
        match message {
            SessionMsg::View { version, view, .. } => {
                *self = Self::restore(version.clone(), view.clone())?;
            }
            SessionMsg::Changes { changes, .. } => {
                for change in changes {
                    if self.version.as_ref() != Some(&change.from) {
                        self.tree = None; self.version = None;
                        return Err("view revision gap; a canonical snapshot is required".into());
                    }
                    let tree = self.tree.as_mut().ok_or("changes arrived without a canonical view")?;
                    for op in &change.ops {
                        if let Err(error) = tree.apply(op) {
                            self.tree = None; self.version = None;
                            return Err(error);
                        }
                    }
                    self.version = Some(change.version.clone());
                }
            }
            SessionMsg::Streams { streams } => { self.streams = streams.iter().map(|stream| (stream.id.clone(), stream.clone())).collect(); }
            SessionMsg::Event { event: SessionEvent::Stream { update }, .. } => match update {
                StreamUpdate::Current { stream } => { self.streams.insert(stream.id.clone(), stream.clone()); }
                StreamUpdate::Append { id, offset, text } => {
                    let stream = self.streams.get_mut(id).ok_or("append arrived without a current stream")?;
                    if stream.text.len() != *offset { return Err("stream byte offset gap; a current value is required".into()); }
                    stream.text.push_str(text);
                }
                StreamUpdate::End { id } => { self.streams.remove(id); }
            },
            _ => return Ok(false),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod receiver_tests {
    use super::*;
    use crate::{SessionMsg, SessionEvent, SubId};
    fn version(rev: u64) -> Version { Version { epoch: "incarnation".into(), rev } }
    fn root() -> Node { Node::section("root").id("root").child(Node::section("a").id("a")) }
    #[test]
    fn colliding_descendants_are_rejected_before_tree_mutation() {
        let mut tree = IndexedTree::new(root());
        let before = tree.snapshot();
        for op in [
            ViewOp::Insert { parent: "root".into(), before: None, node: Node::section("b").id("b").child(Node::section("bad").id("root")) },
            ViewOp::Replace { id: "a".into(), node: Node::section("a").id("a").child(Node::section("bad").id("root")) },
        ] {
            assert!(tree.apply(&op).is_err());
            assert_eq!(tree.snapshot(), before);
        }
    }
    #[test]
    fn restore_checks_generated_addresses_as_well_as_explicit_ones() {
        let root = root().child(Node::section("x")).child(Node::section("explicit").id("root.x.0"));
        assert!(ClientView::restore(version(0), root).is_err());
    }
    #[test]
    fn revision_gap_discards_the_accumulator_and_requires_snapshot() {
        let mut state = ClientView::restore(version(0), root()).unwrap();
        let bad = SessionMsg::Changes { id: SubId(1), changes: vec![Change { from: version(1), version: version(2), ops: vec![] }] };
        assert!(state.receive(&bad).is_err());
        assert!(state.persisted().is_none());
        state.receive(&SessionMsg::View { id: SubId(1), version: version(2), view: root() }).unwrap();
        assert_eq!(state.version(), Some(&version(2)));
    }
    #[test]
    fn utf8_append_offsets_are_bytes_and_never_change_persisted_tree() {
        let mut state = ClientView::restore(version(0), root()).unwrap();
        let before = state.persisted();
        state.receive(&SessionMsg::Streams { streams: vec![Stream { id: "msg.2.text".into(), role: "message.assistant".into(), text: "é".into() }] }).unwrap();
        let append = |offset| SessionMsg::Event { seq: 1, event: SessionEvent::Stream { update: StreamUpdate::Append { id: "msg.2.text".into(), offset, text: "🙂".into() } } };
        assert!(state.receive(&append(1)).is_err());
        state.receive(&append(2)).unwrap();
        assert_eq!(state.streams()[0].text, "é🙂");
        assert_eq!(state.persisted(), before);
    }
    #[test]
    fn a_large_transcript_crosses_chunks_and_restores_into_the_client() {
        let body = "x".repeat(8 * 1024 * 1024 + 1024);
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::text("message.assistant", [crate::view::Span::plain(&body)]).id("msg.1.text")));
        let message = SessionMsg::View { id: SubId(1), version: version(1), view: view.clone() };
        let wire = crate::chunk::encode(&message).unwrap();
        let mut decoder = crate::chunk::Decoder::new();
        for bytes in wire.chunks(4093) { decoder.push(bytes).unwrap(); }
        let received: SessionMsg = crate::chunk::decode(&decoder.next().unwrap().unwrap()).unwrap();
        let mut client = ClientView::default();
        client.receive(&received).unwrap();
        assert_eq!(client.canonical(), Some(view));
    }
    #[test]
    fn canonical_history_cardinality_is_not_a_protocol_limit() {
        let mut root = Node::section("root").id("root");
        root.children = (0..200_001).map(|id| Node::section("message").id(format!("msg.{id}"))).collect();
        crate::view::validate(&root).unwrap();
    }
}
