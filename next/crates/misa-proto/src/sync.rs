//! Canonical view revisions and id-addressed, self-contained changes.
use crate::view::Node;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub epoch: String,
    pub rev: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ViewOp {
    Insert {
        parent: String,
        before: Option<String>,
        node: Node,
    },
    Remove {
        id: String,
    },
    Replace {
        id: String,
        node: Node,
    },
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
    Current {
        stream: Stream,
    },
    Append {
        id: String,
        offset: usize,
        text: String,
    },
    End {
        id: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ViewSync {
    Snapshot {
        version: Version,
        view: Node,
        streams: Vec<Stream>,
    },
    Changes {
        version: Version,
        changes: Vec<Change>,
        streams: Vec<Stream>,
    },
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
        let mut tree = Self {
            root: root.id.clone(),
            nodes: HashMap::new(),
        };
        tree.store(root, None, None, None);
        tree
    }

    fn store(
        &mut self,
        mut node: Node,
        parent: Option<String>,
        previous: Option<String>,
        next: Option<String>,
    ) {
        let children = std::mem::take(&mut node.children);
        let id = node.id.clone();
        self.nodes.insert(
            id.clone(),
            Entry {
                node,
                parent,
                previous,
                next,
                first: children.first().map(|child| child.id.clone()),
                last: children.last().map(|child| child.id.clone()),
            },
        );
        let mut previous = None;
        let mut children = children.into_iter().peekable();
        while let Some(child) = children.next() {
            let next = children.peek().map(|child| child.id.clone());
            let child_id = child.id.clone();
            self.store(child, Some(id.clone()), previous, next);
            previous = Some(child_id);
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.get(id).map(|entry| &entry.node)
    }
    /// Every stored node, in no particular order. Content addressed work such as
    /// finding the blobs a tree refers to does not need document order.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values().map(|entry| &entry.node)
    }
    pub fn parent(&self, id: &str) -> Option<&str> {
        self.nodes.get(id)?.parent.as_deref()
    }
    /// The first child of `id`, if any. Returns an id borrowed from the index.
    pub fn first_child(&self, id: &str) -> Option<&str> {
        self.nodes.get(id)?.first.as_deref()
    }
    /// The last child of `id`, if any. Returns an id borrowed from the index.
    pub fn last_child(&self, id: &str) -> Option<&str> {
        self.nodes.get(id)?.last.as_deref()
    }
    /// The sibling immediately after `id`, if any.
    pub fn next_sibling(&self, id: &str) -> Option<&str> {
        self.nodes.get(id)?.next.as_deref()
    }
    /// The sibling immediately before `id`, if any.
    pub fn previous_sibling(&self, id: &str) -> Option<&str> {
        self.nodes.get(id)?.previous.as_deref()
    }
    pub fn children(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor = self.nodes.get(id).and_then(|entry| entry.first.as_deref());
        while let Some(id) = cursor {
            out.push(id.to_owned());
            cursor = self.nodes.get(id).and_then(|entry| entry.next.as_deref());
        }
        out
    }
    pub fn snapshot(&self) -> Node {
        self.subtree(&self.root).expect("indexed root exists")
    }
    pub fn subtree(&self, id: &str) -> Option<Node> {
        let mut node = self.nodes.get(id)?.node.clone();
        node.children = self
            .children(id)
            .iter()
            .filter_map(|child| self.subtree(child))
            .collect();
        Some(node)
    }

    fn erase(&mut self, id: &str) {
        for child in self.children(id) {
            self.erase(&child);
        }
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
            if node.id.is_empty() {
                return Err("operation node has no identity".into());
            }
            if self.contains(&node.id) && !allowed.contains(&node.id) {
                return Err(format!("duplicate node {}", node.id));
            }
            pending.extend(&node.children);
        }
        Ok(())
    }

    fn check_depth(&self, node: &Node, parent: Option<&str>) -> Result<(), String> {
        let mut depth = 0;
        let mut cursor = parent;
        while let Some(id) = cursor {
            depth += 1;
            cursor = self.parent(id);
        }
        let mut pending = vec![(node, depth)];
        while let Some((node, depth)) = pending.pop() {
            if depth > crate::view::MAX_DEPTH {
                return Err("operation exceeds the document depth bound".into());
            }
            pending.extend(node.children.iter().map(|child| (child, depth + 1)));
            if let crate::view::Kind::List { items, .. } = &node.kind {
                pending.extend(items.iter().flatten().map(|child| (child, depth + 1)));
            }
        }
        Ok(())
    }

    /// Apply one op. A receiver discards its accumulator and resynchronizes on an invalid op.
    pub fn apply(&mut self, op: &ViewOp) -> Result<(), String> {
        match op {
            ViewOp::Insert {
                parent,
                before,
                node,
            } => {
                self.check_depth(node, Some(parent))?;
                self.check_subtree(node, None)?;
                if self.contains(&node.id) {
                    return Err(format!("duplicate node {}", node.id));
                }
                let owner = self
                    .nodes
                    .get(parent)
                    .ok_or_else(|| format!("missing parent {parent}"))?;
                let previous = if let Some(before) = before {
                    let next = self
                        .nodes
                        .get(before)
                        .ok_or_else(|| format!("missing sibling {before}"))?;
                    if next.parent.as_ref() != Some(parent) {
                        return Err("sibling belongs to another parent".into());
                    }
                    next.previous.clone()
                } else {
                    owner.last.clone()
                };
                if let Some(previous) = &previous {
                    self.nodes.get_mut(previous).unwrap().next = Some(node.id.clone());
                } else {
                    self.nodes.get_mut(parent).unwrap().first = Some(node.id.clone());
                }
                if let Some(next) = before {
                    self.nodes.get_mut(next).unwrap().previous = Some(node.id.clone());
                } else {
                    self.nodes.get_mut(parent).unwrap().last = Some(node.id.clone());
                }
                self.store(node.clone(), Some(parent.clone()), previous, before.clone());
            }
            ViewOp::Remove { id } => {
                let entry = self
                    .nodes
                    .get(id)
                    .ok_or_else(|| format!("missing node {id}"))?;
                let parent = entry.parent.clone().ok_or("cannot remove the root")?;
                let previous = entry.previous.clone();
                let next = entry.next.clone();
                if let Some(previous) = &previous {
                    self.nodes.get_mut(previous).unwrap().next = next.clone();
                } else {
                    self.nodes.get_mut(&parent).unwrap().first = next.clone();
                }
                if let Some(next) = &next {
                    self.nodes.get_mut(next).unwrap().previous = previous.clone();
                } else {
                    self.nodes.get_mut(&parent).unwrap().last = previous.clone();
                }
                self.erase(id);
            }
            ViewOp::Replace { id, node } => {
                self.check_depth(node, self.parent(id))?;
                self.check_subtree(node, Some(id))?;
                if node.id != *id {
                    return Err("replacement changes identity".into());
                }
                let entry = self
                    .nodes
                    .get(id)
                    .ok_or_else(|| format!("missing node {id}"))?;
                let (parent, previous, next) = (
                    entry.parent.clone(),
                    entry.previous.clone(),
                    entry.next.clone(),
                );
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

    fn assert_links(tree: &IndexedTree, parent: &str, expected: &[&str]) {
        assert_eq!(tree.first_child(parent), expected.first().copied());
        assert_eq!(tree.last_child(parent), expected.last().copied());
        assert_eq!(tree.children(parent), expected);
        for (index, &id) in expected.iter().enumerate() {
            assert_eq!(tree.parent(id), Some(parent));
            assert_eq!(
                tree.previous_sibling(id),
                index.checked_sub(1).map(|i| expected[i])
            );
            assert_eq!(tree.next_sibling(id), expected.get(index + 1).copied());
        }
        let mut forward = Vec::new();
        let mut cursor = tree.first_child(parent);
        while let Some(id) = cursor {
            forward.push(id);
            cursor = tree.next_sibling(id);
        }
        assert_eq!(forward, expected);
        let mut backward = Vec::new();
        let mut cursor = tree.last_child(parent);
        while let Some(id) = cursor {
            backward.push(id);
            cursor = tree.previous_sibling(id);
        }
        assert_eq!(backward, expected.iter().rev().copied().collect::<Vec<_>>());
    }

    fn assert_absent(tree: &IndexedTree, id: &str) {
        assert!(!tree.contains(id));
        assert_eq!(tree.first_child(id), None);
        assert_eq!(tree.last_child(id), None);
        assert_eq!(tree.next_sibling(id), None);
        assert_eq!(tree.previous_sibling(id), None);
    }

    #[test]
    fn linked_traversal_tracks_inserts_removes_and_nested_replacements() {
        let mut tree = IndexedTree::new(Node::section("root").id("root"));
        assert_links(&tree, "root", &[]);
        assert_eq!(tree.previous_sibling("root"), None);
        assert_eq!(tree.next_sibling("root"), None);
        assert_absent(&tree, "missing");

        for (id, before, expected) in [
            ("a", None, vec!["a"]),                     // append to empty parent
            ("c", None, vec!["a", "c"]),                // append to nonempty parent
            ("b", Some("c"), vec!["a", "b", "c"]),      // insert in middle
            ("z", Some("a"), vec!["z", "a", "b", "c"]), // insert at head
            ("d", None, vec!["z", "a", "b", "c", "d"]), // append at tail
        ] {
            tree.apply(&ViewOp::Insert {
                parent: "root".into(),
                before: before.map(str::to_owned),
                node: Node::section(id).id(id),
            })
            .unwrap();
            assert_links(&tree, "root", &expected);
            assert_links(&tree, id, &[]);
        }
        for (id, expected) in [
            ("z", vec!["a", "b", "c", "d"]), // remove head
            ("b", vec!["a", "c", "d"]),      // remove middle
            ("d", vec!["a", "c"]),           // remove tail
        ] {
            tree.apply(&ViewOp::Remove { id: id.into() }).unwrap();
            assert_links(&tree, "root", &expected);
            assert_absent(&tree, id);
        }

        tree.apply(&ViewOp::Replace {
            id: "a".into(),
            node: Node::section("a")
                .id("a")
                .child(Node::section("u").id("u"))
                .child(Node::section("v").id("v")),
        })
        .unwrap();
        assert_links(&tree, "root", &["a", "c"]);
        assert_links(&tree, "a", &["u", "v"]);
        tree.apply(&ViewOp::Insert {
            parent: "a".into(),
            before: Some("v".into()),
            node: Node::section("w").id("w"),
        })
        .unwrap();
        tree.apply(&ViewOp::Insert {
            parent: "a".into(),
            before: None,
            node: Node::section("x").id("x"),
        })
        .unwrap();
        assert_links(&tree, "a", &["u", "w", "v", "x"]);
        tree.apply(&ViewOp::Replace {
            id: "v".into(),
            node: Node::section("v").id("v").child(Node::section("q").id("q")),
        })
        .unwrap();
        assert_links(&tree, "a", &["u", "w", "v", "x"]);
        assert_links(&tree, "v", &["q"]);
        tree.apply(&ViewOp::Remove { id: "v".into() }).unwrap();
        assert_links(&tree, "a", &["u", "w", "x"]);
        for id in ["v", "q"] {
            assert_absent(&tree, id);
        }
        tree.apply(&ViewOp::Remove { id: "a".into() }).unwrap();
        assert_links(&tree, "root", &["c"]);
        for id in ["a", "u", "w", "x"] {
            assert_absent(&tree, id);
        }
        tree.apply(&ViewOp::Remove { id: "c".into() }).unwrap();
        assert_links(&tree, "root", &[]);
        assert_absent(&tree, "c");
    }

    #[test]
    fn indexed_siblings_keep_order_after_insert_remove_and_replace() {
        let mut tree = IndexedTree::new(
            Node::section("root")
                .child(Node::section("a").id("a"))
                .child(Node::section("c").id("c")),
        );
        tree.apply(&ViewOp::Insert {
            parent: "root".into(),
            before: Some("c".into()),
            node: Node::section("b").id("b"),
        })
        .unwrap();
        tree.apply(&ViewOp::Remove { id: "a".into() }).unwrap();
        tree.apply(&ViewOp::Replace {
            id: "b".into(),
            node: Node::section("b")
                .id("b")
                .child(Node::section("inside").id("inside")),
        })
        .unwrap();
        assert_eq!(tree.children("root"), vec!["b", "c"]);
        assert_eq!(tree.snapshot().children[0].children[0].id, "inside");
        assert_eq!(tree.len(), 4);
    }
}

/// Give immutable content nodes addresses under their nearest semantic owner.
/// List members whose position can move must already carry their domain identity.
pub fn address(node: &mut Node) {
    if node.id.is_empty() {
        node.id = node.role.clone();
    }
    let mut roles = std::collections::BTreeMap::<String, usize>::new();
    for child in &mut node.children {
        let occurrence = roles.entry(child.role.clone()).or_default();
        if child.id.is_empty() {
            child.id = format!("{}.{}.{}", node.id, child.role, occurrence);
        }
        *occurrence += 1;
        address(child);
    }
}

#[cfg(test)]
mod receiver_tests {
    use super::*;
    use crate::observation::{Content, Document, Snapshot};
    fn version(rev: u64) -> Version {
        Version {
            epoch: "incarnation".into(),
            rev,
        }
    }
    fn root() -> Node {
        Node::section("root")
            .id("root")
            .child(Node::section("a").id("a"))
    }
    #[test]
    fn colliding_descendants_are_rejected_before_tree_mutation() {
        let mut tree = IndexedTree::new(root());
        let before = tree.snapshot();
        for op in [
            ViewOp::Insert {
                parent: "root".into(),
                before: None,
                node: Node::section("b")
                    .id("b")
                    .child(Node::section("bad").id("root")),
            },
            ViewOp::Replace {
                id: "a".into(),
                node: Node::section("a")
                    .id("a")
                    .child(Node::section("bad").id("root")),
            },
        ] {
            assert!(tree.apply(&op).is_err());
            assert_eq!(tree.snapshot(), before);
        }
    }
    #[test]
    fn restore_checks_generated_addresses_as_well_as_explicit_ones() {
        let root = root()
            .child(Node::section("x"))
            .child(Node::section("explicit").id("root.x.0"));
        let mut root = root;
        address(&mut root);
        assert!(crate::view::validate(&root).is_err());
    }
    #[test]
    fn a_large_transcript_crosses_chunks_and_restores_into_the_client() {
        let body = "x".repeat(8 * 1024 * 1024 + 1024);
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::text("message.assistant", [crate::view::Span::plain(&body)]).id("msg.1.text"),
            ),
        );
        let message = Snapshot {
            position: 1,
            members: std::collections::BTreeMap::from([(
                "conversation".into(),
                Content::Document(Document {
                    version: version(1),
                    tree: view.clone(),
                    streams: vec![],
                }),
            )]),
        };
        let wire = crate::chunk::encode(&message).unwrap();
        let mut decoder = crate::chunk::Decoder::new();
        for bytes in wire.chunks(4093) {
            decoder.push(bytes).unwrap();
        }
        let received: Snapshot = crate::chunk::decode(&decoder.next().unwrap().unwrap()).unwrap();
        assert_eq!(received, message);
        let Content::Document(document) = &received.members["conversation"] else {
            panic!("document")
        };
        crate::view::validate(&document.tree).unwrap();
        assert_eq!(IndexedTree::new(document.tree.clone()).snapshot(), view);
    }
    #[test]
    fn canonical_history_cardinality_is_not_a_protocol_limit() {
        let mut root = Node::section("root").id("root");
        root.children = (0..200_001)
            .map(|id| Node::section("message").id(format!("msg.{id}")))
            .collect();
        crate::view::validate(&root).unwrap();
    }
    #[test]
    fn operation_depth_includes_its_existing_ancestors() {
        let mut root = Node::section("leaf").id("leaf");
        for depth in 0..crate::view::MAX_DEPTH {
            root = Node::section("layer")
                .id(format!("layer.{depth}"))
                .child(root);
        }
        let mut tree = IndexedTree::new(root);
        let before = tree.snapshot();
        let insert = ViewOp::Insert {
            parent: "leaf".into(),
            before: None,
            node: Node::section("child").id("child"),
        };
        assert!(tree.apply(&insert).is_err());
        let replace = ViewOp::Replace {
            id: "leaf".into(),
            node: Node::section("leaf")
                .id("leaf")
                .child(Node::section("child").id("child")),
        };
        assert!(tree.apply(&replace).is_err());
        assert_eq!(tree.snapshot(), before);
    }
}
