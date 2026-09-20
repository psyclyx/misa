//! Database patch owners: structure is explicit; content is rebuilt only for declared inputs.
use crate::views::{self, Section};
use misa_proto::sync::{Change, IndexedTree, Version, ViewOp, ViewSync, address};
use misa_proto::view::Node;
use misa_reframe::Change as DbChange;
use misa_value::{Op, Path, Seg, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Retention and connection capacity are revision units, never individual operations.
pub const RETAINED_REVISIONS: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub content_builds: u64,
    pub structural_ops: u64,
    pub encoded_op_bytes: u64,
    pub snapshot_nodes: u64,
}

pub struct Canonical {
    pub tree: IndexedTree,
    pub version: Version,
    history: VecDeque<Change>,
    slots: BTreeMap<String, String>,
    order: Vec<String>,
    memo: BTreeMap<String, Vec<Option<Value>>>,
    groups: Vec<(String, usize)>,
    queue_count: usize,
    pub work: Work,
}

impl Canonical {
    pub fn new(db: &Value, sections: &[Section], epoch: String) -> Self {
        let view = views::document(db, sections);
        let mut order = vec!["transcript".into()];
        order.extend(sections.iter().map(|section| section.namespace.clone()));
        order.extend(
            [
                "panel",
                "notices",
                "queue",
                "attachments",
                "composer",
                "turn",
            ]
            .map(str::to_owned),
        );
        let mut slots = BTreeMap::new();
        for child in &view.children {
            let slot = match child.role.as_str() {
                "panel" => "panel",
                other => other,
            };
            slots.insert(slot.to_owned(), child.id.clone());
        }
        let mut state = Self {
            tree: IndexedTree::new(view),
            version: Version { epoch, rev: 0 },
            history: VecDeque::new(),
            slots,
            order,
            memo: BTreeMap::new(),
            groups: vec![],
            queue_count: db
                .get("session")
                .and_then(|session| session.get("queue"))
                .and_then(Value::as_list)
                .map_or(0, <[Value]>::len),
            work: Work::default(),
        };
        state.reset_groups();
        state
    }

    fn reset_groups(&mut self) {
        self.groups = self
            .tree
            .children("transcript")
            .into_iter()
            .filter(|id| id.starts_with("group."))
            .map(|id| {
                let count = self.tree.children(&id).len()
                    - usize::from(self.tree.contains(&format!("{id}.footer")));
                (id, count)
            })
            .collect();
    }
    fn emit(&mut self, mut op: ViewOp, out: &mut Vec<ViewOp>) {
        match &mut op {
            ViewOp::Insert { node, .. } | ViewOp::Replace { node, .. } => address(node),
            _ => {}
        }
        self.tree
            .apply(&op)
            .expect("database owner emitted an applicable view op");
        self.work.structural_ops += 1;
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&op, &mut encoded).expect("a view op encodes");
        self.work.encoded_op_bytes += encoded.len() as u64;
        out.push(op);
    }

    fn slot(&mut self, name: &str, node: Option<Node>, out: &mut Vec<ViewOp>) {
        if name == "queue" {
            self.queue_count = node
                .as_ref()
                .map_or(0, |node| node.children.len().saturating_sub(1));
        }
        let previous = self.slots.get(name).cloned();
        if let (Some(previous), Some(node)) = (&previous, &node) {
            if previous == &node.id {
                self.emit(
                    ViewOp::Replace {
                        id: previous.clone(),
                        node: node.clone(),
                    },
                    out,
                );
                return;
            }
        }
        if let Some(id) = previous {
            self.emit(ViewOp::Remove { id }, out);
            self.slots.remove(name);
        }
        if let Some(node) = node {
            let at = self
                .order
                .iter()
                .position(|slot| slot == name)
                .expect("declared root owner");
            let before = self.order[at + 1..]
                .iter()
                .find_map(|slot| self.slots.get(slot))
                .cloned();
            self.slots.insert(name.to_owned(), node.id.clone());
            self.emit(
                ViewOp::Insert {
                    parent: "session".into(),
                    before,
                    node,
                },
                out,
            );
        }
    }

    fn changed(&mut self, name: &str, db: &Value, paths: &[Path]) -> bool {
        let values = paths
            .iter()
            .map(|path| db.get_path(path).cloned())
            .collect::<Vec<_>>();
        let same = self.memo.get(name).is_some_and(|previous| {
            previous.len() == values.len()
                && previous.iter().zip(&values).all(|(a, b)| match (a, b) {
                    (Some(a), Some(b)) => a.same(b),
                    (None, None) => true,
                    _ => false,
                })
        });
        if same {
            return false;
        }
        self.memo.insert(name.to_owned(), values);
        self.work.content_builds += 1;
        true
    }

    fn append_message(&mut self, message: &Value, out: &mut Vec<ViewOp>) {
        let nodes = views::message_nodes(message);
        if nodes.is_empty() {
            return;
        }
        self.work.content_builds += 1;
        if self.tree.contains("transcript.transcript.empty.0") {
            self.emit(
                ViewOp::Remove {
                    id: "transcript.transcript.empty.0".into(),
                },
                out,
            );
        }
        if message.get("role").and_then(Value::as_str) == Some("user") || self.groups.is_empty() {
            let seq = message.get("seq").and_then(Value::as_i64).unwrap_or(0);
            let id = format!("group.{seq}");
            let mut group = Node::section("message.group").id(&id);
            group.children.extend(nodes);
            self.emit(
                ViewOp::Insert {
                    parent: "transcript".into(),
                    before: None,
                    node: views::finish_group(
                        group,
                        message.get("attempt").cloned(),
                        Some(message),
                    ),
                },
                out,
            );
            self.groups.push((id, 1));
        } else {
            let (id, count) = self.groups.last_mut().unwrap();
            *count += 1;
            let id = id.clone();
            let footer = format!("{id}.footer");
            let before = self.tree.contains(&footer).then_some(footer.clone());
            for node in nodes {
                self.emit(
                    ViewOp::Insert {
                        parent: id.clone(),
                        before: before.clone(),
                        node,
                    },
                    out,
                );
            }
            // A settled attempt replaces the opener's separator so the group keeps
            // exactly one footer and the timing facts stay last.
            if message.get("role").and_then(Value::as_str) == Some("assistant")
                && let Some(attempt) = message.get("attempt")
            {
                if self.tree.contains(&footer) {
                    self.emit(ViewOp::Remove { id: footer.clone() }, out);
                }
                self.emit(
                    ViewOp::Insert {
                        parent: id.clone(),
                        before: None,
                        node: views::attempt_footer(&id, attempt),
                    },
                    out,
                );
            }
        }
    }

    fn queue_patch(&mut self, change: &DbChange, path: &Path, op: &Op, out: &mut Vec<ViewOp>) {
        match (path.segments(), op) {
            ([Seg::Key(_), Seg::Key(_)], Op::Append(row)) => {
                if self.tree.contains("queue") {
                    self.emit(
                        ViewOp::Insert {
                            parent: "queue".into(),
                            before: None,
                            node: views::queue_item(row),
                        },
                        out,
                    );
                    self.queue_count += 1;
                    self.emit(
                        ViewOp::Replace {
                            id: "queue.count".into(),
                            node: views::count_fact("queue", self.queue_count),
                        },
                        out,
                    );
                } else {
                    let db = Value::map([(
                        "session",
                        Value::map([("queue", Value::list([row.clone()]))]),
                    )]);
                    self.slot("queue", views::queue(&db), out);
                }
            }
            ([Seg::Key(_), Seg::Key(_), Seg::Index(index)], Op::Delete) => {
                if let Some(id) = self
                    .tree
                    .children("queue")
                    .get(*index as usize + 1)
                    .cloned()
                {
                    self.emit(ViewOp::Remove { id }, out);
                    self.queue_count -= 1;
                }
                if self.queue_count == 0 {
                    self.slot("queue", None, out);
                } else {
                    self.emit(
                        ViewOp::Replace {
                            id: "queue.count".into(),
                            node: views::count_fact("queue", self.queue_count),
                        },
                        out,
                    );
                }
            }
            _ => self.slot("queue", views::queue(&change.after), out),
        }
    }

    pub fn advance(&mut self, db: &Value, changes: &[DbChange], sections: &[Section], rev: u64) {
        let mut out = Vec::new();
        for change in changes {
            let mut dirty = BTreeSet::new();
            let mut calls = BTreeSet::new();
            let mut messages = BTreeSet::new();
            // Membership rewrites may move turn boundaries. They explicitly replace that
            // structural owner once; ordinary settled appends never take this path.
            let rewrite_transcript =
                change
                    .patches
                    .iter()
                    .any(|(path, op)| match path.segments() {
                        [Seg::Key(root)] if root == "messages" => {
                            !matches!(op, Op::Append(_) | Op::AppendAll(_))
                        }
                        [Seg::Key(root), Seg::Index(_)] if root == "messages" => true,
                        _ => false,
                    });
            for (path, op) in &change.patches {
                let segments = path.segments();
                match segments {
                    [Seg::Key(root), ..] if root == "messages" && rewrite_transcript => {}
                    [Seg::Key(root), tail @ ..] if root == "messages" => match (tail, op) {
                        ([], Op::Append(message)) => self.append_message(message, &mut out),
                        ([], Op::AppendAll(messages)) => {
                            for message in messages {
                                self.append_message(message, &mut out);
                            }
                        }
                        ([Seg::Index(index), Seg::Key(key), Seg::Index(call), ..], _)
                            if key == "calls" =>
                        {
                            calls.insert((*index as usize, *call as usize));
                        }
                        ([Seg::Index(index), ..], _) => {
                            messages.insert(*index as usize);
                        }
                        _ => {
                            self.slot(
                                "transcript",
                                Some(views::transcript(&change.after)),
                                &mut out,
                            );
                            self.reset_groups();
                        }
                    },
                    [Seg::Key(root), ..] if root == "panel" => {
                        dirty.insert("panel".to_owned());
                    }
                    [Seg::Key(root), ..] if root == "notices" => {
                        dirty.insert("notices".to_owned());
                    }
                    [Seg::Key(root), Seg::Key(key), ..] if root == "session" => {
                        match key.as_str() {
                            "status" => {
                                dirty.insert("turn".to_owned());
                            }
                            "queue" => self.queue_patch(change, path, op, &mut out),
                            "attachments" => {
                                dirty.insert("attachments".to_owned());
                            }
                            _ => {}
                        }
                    }
                    [Seg::Key(root)] if root == "session" => {
                        dirty.extend(["turn", "queue", "attachments"].map(str::to_owned));
                    }
                    _ => {}
                }
                for (index, section) in sections.iter().enumerate() {
                    if section.inputs.iter().any(|input| overlaps(path, input)) {
                        dirty.insert(format!("plugin:{index}"));
                    }
                }
            }
            let db = &change.after;
            if rewrite_transcript {
                self.slot("transcript", Some(views::transcript(db)), &mut out);
                self.reset_groups();
            }
            for (index, call) in calls {
                let path = Path::parse(&format!("messages[{index}].calls[{call}]")).unwrap();
                if !self.changed(&format!("call:{index}:{call}"), db, &[path]) {
                    continue;
                }
                if let Some(row) = db
                    .get("messages")
                    .and_then(Value::as_list)
                    .and_then(|rows| rows.get(index))
                    .and_then(|message| message.get("calls"))
                    .and_then(Value::as_list)
                    .and_then(|calls| calls.get(call))
                {
                    let seq = db
                        .get("messages")
                        .and_then(Value::as_list)
                        .and_then(|rows| rows.get(index))
                        .and_then(|message| message.get("seq"))
                        .and_then(Value::as_i64)
                        .unwrap_or(index as i64);
                    let node = views::call_node(&format!("msg.{seq}"), row, call);
                    self.emit(
                        ViewOp::Replace {
                            id: node.id.clone(),
                            node,
                        },
                        &mut out,
                    );
                }
            }
            for index in messages {
                let path = Path::parse(&format!("messages[{index}]")).unwrap();
                if !self.changed(&format!("message:{index}"), db, &[path]) {
                    continue;
                }
                if let Some(row) = db
                    .get("messages")
                    .and_then(Value::as_list)
                    .and_then(|rows| rows.get(index))
                {
                    if let Some(node) = views::message_node(row) {
                        self.emit(
                            ViewOp::Replace {
                                id: node.id.clone(),
                                node,
                            },
                            &mut out,
                        );
                    }
                }
            }
            for owner in dirty {
                if let Some(index) = owner
                    .strip_prefix("plugin:")
                    .and_then(|index| index.parse::<usize>().ok())
                {
                    let section = &sections[index];
                    if self.changed(&owner, db, &section.inputs) {
                        let node = views::section_node(section, db);
                        if self.tree.subtree(&section.namespace).as_ref() != Some(&node) {
                            self.slot(&section.namespace, Some(node), &mut out);
                        }
                    }
                    continue;
                }
                let inputs: &[&str] = match owner.as_str() {
                    "turn" => &["session.status"],
                    "panel" => &["panel"],
                    "notices" => &["notices"],
                    "queue" => &["session.queue"],
                    "attachments" => &["session.attachments"],
                    _ => unreachable!(),
                };
                let paths = inputs
                    .iter()
                    .map(|path| Path::parse(path).unwrap())
                    .collect::<Vec<_>>();
                if !self.changed(&owner, db, &paths) {
                    continue;
                }
                let node = match owner.as_str() {
                    "turn" => views::cancel(db),
                    "panel" => views::panel(db),
                    "notices" => views::notices(db),
                    "queue" => views::queue(db),
                    "attachments" => views::attachments(db),
                    _ => unreachable!(),
                };
                self.slot(&owner, node, &mut out);
            }
            let label = Some(
                db.get("session")
                    .and_then(|session| session.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("session")
                    .to_owned(),
            );
            if self
                .tree
                .node("session")
                .is_some_and(|node| node.label != label)
            {
                let mut root = self.tree.snapshot();
                self.work.snapshot_nodes += self.tree.len() as u64;
                root.label = label;
                self.emit(
                    ViewOp::Replace {
                        id: "session".into(),
                        node: root,
                    },
                    &mut out,
                );
            }
        }
        let previous = self.version.clone();
        self.version.rev = rev;
        self.history.push_back(Change {
            from: previous,
            version: self.version.clone(),
            ops: out,
        });
        while self.history.len() > RETAINED_REVISIONS {
            self.history.pop_front();
        }
        // Rebuild remains the specification, never the production update mechanism.
        if cfg!(test) || (cfg!(debug_assertions) && self.tree.len() <= 128) {
            assert_eq!(
                self.tree.snapshot(),
                views::document(db, sections),
                "incremental view differs from database rebuild"
            );
        }
    }

    pub fn sync(
        &mut self,
        since: Option<&Version>,
        streams: Vec<misa_proto::sync::Stream>,
    ) -> ViewSync {
        if let Some(since) = since {
            if since == &self.version {
                return ViewSync::Changes {
                    version: self.version.clone(),
                    changes: vec![],
                    streams,
                };
            }
            if let Some(at) = self.history.iter().position(|change| &change.from == since) {
                return ViewSync::Changes {
                    version: self.version.clone(),
                    changes: self.history.iter().skip(at).cloned().collect(),
                    streams,
                };
            }
        }
        self.work.snapshot_nodes += self.tree.len() as u64;
        ViewSync::Snapshot {
            version: self.version.clone(),
            view: self.tree.snapshot(),
            streams,
        }
    }
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.segments().starts_with(b.segments()) || b.segments().starts_with(a.segments())
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::view::Kind;
    fn advance(
        view: &mut Canonical,
        db: &mut Value,
        patches: Vec<(Path, Op)>,
        sections: &[Section],
    ) {
        let after = misa_value::apply(db, &patches).unwrap();
        let change = DbChange {
            before: db.clone(),
            after: after.clone(),
            patches,
        };
        *db = after;
        view.advance(db, &[change], sections, view.version.rev + 1);
    }
    fn patch(path: &str, op: Op) -> (Path, Op) {
        (Path::parse(path).unwrap(), op)
    }
    fn row(id: i64) -> Value {
        Value::map([
            ("id", Value::Int(id)),
            ("text", Value::str("queued")),
            ("attachments", Value::list([])),
        ])
    }
    #[test]
    fn sequential_queue_deletes_keep_the_surviving_identity() {
        let mut db = views::initial_state("test", "p", "m", 0);
        let mut view = Canonical::new(&db, &[], "epoch".into());
        advance(
            &mut view,
            &mut db,
            vec![
                patch("session.queue", Op::Append(row(1))),
                patch("session.queue", Op::Append(row(2))),
                patch("session.queue", Op::Append(row(3))),
            ],
            &[],
        );
        advance(
            &mut view,
            &mut db,
            vec![
                patch("session.queue[0]", Op::Delete),
                patch("session.queue[0]", Op::Delete),
            ],
            &[],
        );
        assert_eq!(view.tree.children("queue"), vec!["queue.count", "queue.3"]);
    }
    #[test]
    fn retention_catches_up_atomically_or_sends_the_current_snapshot() {
        let mut db = views::initial_state("test", "p", "m", 0);
        let mut view = Canonical::new(&db, &[], "epoch".into());
        let original = view.version.clone();
        for n in 0..64 {
            advance(
                &mut view,
                &mut db,
                vec![patch(
                    "session.status",
                    Op::Set(Value::str(format!("m{n}"))),
                )],
                &[],
            );
        }
        match view.sync(Some(&original), vec![]) {
            ViewSync::Changes {
                changes, version, ..
            } => {
                assert_eq!(changes.len(), 64);
                assert_eq!(version.rev, 64);
                assert_eq!(changes[0].from, original);
            }
            _ => panic!("retained history must catch up"),
        }
        advance(
            &mut view,
            &mut db,
            vec![patch("session.status", Op::Set(Value::str("last")))],
            &[],
        );
        for since in [
            original,
            Version {
                epoch: "previous process".into(),
                rev: 65,
            },
        ] {
            assert!(matches!(
                view.sync(Some(&since), vec![]),
                ViewSync::Snapshot {
                    version: Version { rev: 65, .. },
                    ..
                }
            ));
        }
        let current = view.version.clone();
        assert!(
            matches!(view.sync(Some(&current), vec![]), ViewSync::Changes {changes,..} if changes.is_empty())
        );
    }
    #[test]
    fn unrelated_patches_do_not_rebuild_declared_plugin_content() {
        let builds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = builds.clone();
        let sections = [Section {
            namespace: "plugin.counter".into(),
            inputs: vec![Path::parse("session.model").unwrap()],
            build: std::sync::Arc::new(move |_| {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Node::section("counter").id("counter"))
            }),
        }];
        let mut db = views::initial_state("test", "p", "m", 0);
        let mut view = Canonical::new(&db, &sections, "epoch".into());
        let work = view.work.clone();
        advance(
            &mut view,
            &mut db,
            vec![patch("session.queue_seq", Op::Set(Value::Int(1)))],
            &sections,
        );
        assert_eq!(view.work, work);
        // The test-only differential oracle invokes the rebuild once; production did not.
        assert_eq!(builds.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    fn message(seq: i64, role: &str) -> Value {
        Value::map([
            ("seq", Value::Int(seq)),
            ("role", Value::str(role)),
            ("text", Value::str("x".repeat(64))),
            ("state", Value::str("done")),
        ])
    }
    #[test]
    fn membership_rewrites_and_root_metadata_still_match_the_oracle() {
        let mut db = views::initial_state("test", "p", "m", 0);
        let mut view = Canonical::new(&db, &[], "epoch".into());
        advance(
            &mut view,
            &mut db,
            vec![patch(
                "messages",
                Op::AppendAll(vec![
                    message(1, "user"),
                    message(2, "assistant"),
                    message(3, "user"),
                    message(4, "assistant"),
                ]),
            )],
            &[],
        );
        advance(
            &mut view,
            &mut db,
            vec![
                patch("messages[0]", Op::Delete),
                patch("messages[1]", Op::Delete),
            ],
            &[],
        );
        assert!(!view.tree.contains("msg.1"));
        assert!(!view.tree.contains("msg.3"));
        advance(
            &mut view,
            &mut db,
            vec![patch("session.id", Op::Set(Value::str("renamed")))],
            &[],
        );
        assert_eq!(
            view.tree.node("session").unwrap().label.as_deref(),
            Some("renamed")
        );
    }
    #[test]
    fn settled_append_work_does_not_grow_with_history() {
        let mut results = Vec::new();
        for history in [128, 256] {
            let mut runs = Vec::new();
            for _ in 0..6 {
                let mut db = views::initial_state("test", "p", "m", 0);
                db = misa_value::apply_one(
                    &db,
                    &Path::parse("messages").unwrap(),
                    &Op::Set(Value::list(
                        (1..=history).map(|seq| message(seq, "assistant")),
                    )),
                )
                .unwrap();
                let mut view = Canonical::new(&db, &[], "epoch".into());
                let before = view.work.clone();
                advance(
                    &mut view,
                    &mut db,
                    vec![patch(
                        "messages",
                        Op::Append(message(history + 1, "assistant")),
                    )],
                    &[],
                );
                let work = &view.work;
                assert_eq!(work.content_builds - before.content_builds, 1);
                assert_eq!(work.structural_ops - before.structural_ops, 1);
                assert_eq!(work.snapshot_nodes - before.snapshot_nodes, 0);
                runs.push(work.encoded_op_bytes - before.encoded_op_bytes);
            }
            assert!(runs.iter().all(|bytes| bytes == &runs[0]));
            results.push(runs[0]);
            eprintln!(
                "history={history} repetitions=6 content_builds=1 view_ops=2 encoded_bytes={}",
                runs[0]
            );
        }
        assert!(
            results[1] <= results[0] + 8,
            "only integer-width encoding may vary"
        );
    }
    #[test]
    fn aggregate_updates_coalesce_repeated_row_patches_and_membership_changes() {
        let mut db = views::initial_state("test", "p", "m", 0);
        let registry = std::sync::Arc::new(
            crate::indicators::subscriptions(crate::agent::registry()).subscription(
                crate::indicators::MODEL_QUERY,
                crate::indicators::builtins().subscription(),
            ),
        );
        let sections = [crate::indicators::builtins().section(registry)];
        let mut view = Canonical::new(&db, &sections, "epoch".into());
        let row = |cost| {
            Value::map([
                ("cost_micros", Value::Int(cost)),
                ("input_tokens", Value::Int(10)),
            ])
        };
        advance(
            &mut view,
            &mut db,
            vec![patch("attempts", Op::AppendAll(vec![row(3), row(7)]))],
            &sections,
        );
        advance(
            &mut view,
            &mut db,
            vec![
                patch("attempts[0].cost_micros", Op::Set(Value::Int(5))),
                patch("attempts[0].input_tokens", Op::Set(Value::Int(20))),
            ],
            &sections,
        );
        assert_eq!(
            view.tree
                .node("presentation.status.cost.value.money.0")
                .unwrap()
                .kind,
            Kind::Fact {
                value: Value::Int(12)
            }
        );
        assert_eq!(
            view.tree
                .node("presentation.status.session.value.tokens.0")
                .unwrap()
                .kind,
            Kind::Fact {
                value: Value::Int(30)
            }
        );
        advance(
            &mut view,
            &mut db,
            vec![patch("attempts[0]", Op::Delete)],
            &sections,
        );
        assert_eq!(
            view.tree
                .node("presentation.status.cost.value.money.0")
                .unwrap()
                .kind,
            Kind::Fact {
                value: Value::Int(7)
            }
        );
        assert_eq!(
            view.tree
                .node("presentation.status.session.value.tokens.0")
                .unwrap()
                .kind,
            Kind::Fact {
                value: Value::Int(10)
            }
        );
    }
}
