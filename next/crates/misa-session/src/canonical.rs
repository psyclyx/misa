//! Database patch owners: structure is explicit; content is rebuilt only for declared inputs.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use misa_proto::sync::{Change, IndexedTree, Version, ViewOp, ViewSync, address};
use misa_proto::view::{Kind, Node, Span};
use misa_reframe::Change as DbChange;
use misa_value::{Op, Path, Seg, Value};
use crate::views::{self, Section};

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
    spend: i64,
    context: i64,
    pub work: Work,
}

impl Canonical {
    pub fn new(db: &Value, sections: &[Section], epoch: String) -> Self {
        let view = views::document(db, sections);
        let mut order = vec!["header".into(), "transcript".into()];
        order.extend(sections.iter().map(|section| format!("plugin.{}", section.plugin)));
        order.extend(["panel", "notices", "queue", "attachments", "composer", "turn"].map(str::to_owned));
        let mut slots = BTreeMap::new();
        for child in &view.children {
            let slot = match child.role.as_str() { "session.header" => "header", "panel" => "panel", other => other };
            slots.insert(slot.to_owned(), child.id.clone());
        }
        let mut state = Self { tree: IndexedTree::new(view), version: Version { epoch, rev: 0 },
            history: VecDeque::new(), slots, order, memo: BTreeMap::new(), groups: vec![],
            spend: 0, context: 0, work: Work::default() };
        state.reset_groups();
        state.reset_totals(db);
        state
    }

    fn reset_groups(&mut self) {
        self.groups = self.tree.children("transcript").into_iter().filter(|id| id.starts_with("group."))
            .map(|id| { let count = self.tree.children(&id).len().saturating_sub(2); (id, count) }).collect();
    }
    fn reset_totals(&mut self, db: &Value) {
        let rows = db.get("attempts").and_then(Value::as_list).unwrap_or(&[]);
        self.spend = rows.iter().map(cost).sum();
        self.context = rows.last().map(tokens).unwrap_or(0);
    }

    fn emit(&mut self, mut op: ViewOp, out: &mut Vec<ViewOp>) {
        match &mut op { ViewOp::Insert { node, .. } | ViewOp::Replace { node, .. } => address(node), _ => {} }
        self.tree.apply(&op).expect("database owner emitted an applicable view op");
        self.work.structural_ops += 1;
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&op, &mut encoded).expect("a view op encodes");
        self.work.encoded_op_bytes += encoded.len() as u64;
        out.push(op);
    }

    fn slot(&mut self, name: &str, node: Option<Node>, out: &mut Vec<ViewOp>) {
        let previous = self.slots.get(name).cloned();
        if let (Some(previous), Some(node)) = (&previous, &node) {
            if previous == &node.id {
                self.emit(ViewOp::Replace { id: previous.clone(), node: node.clone() }, out);
                return;
            }
        }
        if let Some(id) = previous { self.emit(ViewOp::Remove { id }, out); self.slots.remove(name); }
        if let Some(node) = node {
            let at = self.order.iter().position(|slot| slot == name).expect("declared root owner");
            let before = self.order[at + 1..].iter().find_map(|slot| self.slots.get(slot)).cloned();
            self.slots.insert(name.to_owned(), node.id.clone());
            self.emit(ViewOp::Insert { parent: "session".into(), before, node }, out);
        }
    }

    fn changed(&mut self, name: &str, db: &Value, paths: &[Path]) -> bool {
        let values = paths.iter().map(|path| db.get_path(path).cloned()).collect::<Vec<_>>();
        let same = self.memo.get(name).is_some_and(|previous| previous.len() == values.len() && previous.iter().zip(&values)
            .all(|(a, b)| match (a, b) { (Some(a), Some(b)) => a.same(b), (None, None) => true, _ => false }));
        if same { return false; }
        self.memo.insert(name.to_owned(), values);
        self.work.content_builds += 1;
        true
    }

    fn append_message(&mut self, message: &Value, out: &mut Vec<ViewOp>) {
        let Some(node) = views::message_node(message) else { return };
        self.work.content_builds += 1;
        if self.tree.contains("transcript.transcript.empty.0") {
            self.emit(ViewOp::Remove { id: "transcript.transcript.empty.0".into() }, out);
        }
        if message.get("role").and_then(Value::as_str) == Some("user") || self.groups.is_empty() {
            let seq = message.get("seq").and_then(Value::as_i64).unwrap_or(0);
            let id = format!("group.{seq}");
            let group = Node::section("message.group").id(&id).child(
                Node::text("message.group.header", [Span::plain("Conversation turn")]).id(format!("{id}.header")),
            ).child(node);
            self.emit(ViewOp::Insert { parent: "transcript".into(), before: None, node: views::finish_group(group) }, out);
            self.groups.push((id, 1));
        } else {
            let (id, count) = self.groups.last_mut().unwrap();
            *count += 1;
            let (id, count) = (id.clone(), *count);
            self.emit(ViewOp::Insert { parent: id.clone(), before: Some(format!("{id}.footer")), node }, out);
            let footer = Node::new("message.group.footer", Kind::Fact { value: Value::Int(count as i64) })
                .id(format!("{id}.footer")).label("messages");
            self.emit(ViewOp::Replace { id: footer.id.clone(), node: footer }, out);
        }
    }

    fn queue_patch(&mut self, change: &DbChange, path: &Path, op: &Op, out: &mut Vec<ViewOp>) {
        match (path.segments(), op) {
            ([Seg::Key(_), Seg::Key(_)], Op::Append(row)) => {
                if self.tree.contains("queue") {
                    self.emit(ViewOp::Insert { parent: "queue".into(), before: None, node: views::queue_item(row) }, out);
                } else {
                    let db = Value::map([("session", Value::map([("queue", Value::list([row.clone()]))]))]);
                    self.slot("queue", views::queue(&db), out);
                }
            }
            ([Seg::Key(_), Seg::Key(_), Seg::Index(index)], Op::Delete) => {
                if let Some(id) = self.tree.children("queue").get(*index as usize).cloned() {
                    self.emit(ViewOp::Remove { id }, out);
                }
                if self.tree.contains("queue") && self.tree.children("queue").is_empty() { self.slot("queue", None, out); }
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
            for (path, op) in &change.patches {
                let segments = path.segments();
                match segments {
                    [Seg::Key(root), tail @ ..] if root == "messages" => match (tail, op) {
                        ([], Op::Append(message)) => self.append_message(message, &mut out),
                        ([], Op::AppendAll(messages)) => for message in messages { self.append_message(message, &mut out); },
                        ([Seg::Index(index), Seg::Key(key), Seg::Index(call), ..], _) if key == "calls" => { calls.insert((*index as usize, *call as usize)); }
                        ([Seg::Index(index), ..], _) => { messages.insert(*index as usize); }
                        _ => { self.slot("transcript", Some(views::transcript(&change.after)), &mut out); self.reset_groups(); }
                    },
                    [Seg::Key(root), tail @ ..] if root == "attempts" => {
                        match (tail, op) {
                            ([], Op::Append(row)) => { self.spend += cost(row); self.context = tokens(row); }
                            ([], Op::AppendAll(rows)) => { self.spend += rows.iter().map(cost).sum::<i64>(); if let Some(row) = rows.last() { self.context = tokens(row); } }
                            ([Seg::Index(index), ..], _) => {
                                let before = change.before.get("attempts").and_then(Value::as_list).unwrap_or(&[]);
                                let after = change.after.get("attempts").and_then(Value::as_list).unwrap_or(&[]);
                                self.spend += after.get(*index as usize).map(cost).unwrap_or(0) - before.get(*index as usize).map(cost).unwrap_or(0);
                                self.context = after.last().map(tokens).unwrap_or(0);
                            }
                            _ => self.reset_totals(&change.after),
                        }
                        dirty.insert("header".to_owned());
                    }
                    [Seg::Key(root), ..] if root == "panel" => { dirty.insert("panel".to_owned()); }
                    [Seg::Key(root), ..] if root == "notices" => { dirty.insert("notices".to_owned()); }
                    [Seg::Key(root), Seg::Key(key), ..] if root == "session" => match key.as_str() {
                        "provider" | "model" | "status" | "turn" => { dirty.insert("header".to_owned()); if key == "status" { dirty.insert("turn".to_owned()); } }
                        "queue" => self.queue_patch(change, path, op, &mut out),
                        "attachments" => { dirty.insert("attachments".to_owned()); }
                        _ => {}
                    },
                    [Seg::Key(root)] if root == "session" => { dirty.extend(["header", "turn", "queue", "attachments"].map(str::to_owned)); }
                    _ => {}
                }
                for (index, section) in sections.iter().enumerate() {
                    if section.inputs.iter().any(|input| overlaps(path, input)) { dirty.insert(format!("plugin:{index}")); }
                }
            }
            let db = &change.after;
            for (index, call) in calls {
                if let Some(row) = db.get("messages").and_then(Value::as_list).and_then(|rows| rows.get(index))
                    .and_then(|message| message.get("calls")).and_then(Value::as_list).and_then(|calls| calls.get(call)) {
                    let node = views::call_node(row, call);
                    self.work.content_builds += 1;
                    self.emit(ViewOp::Replace { id: node.id.clone(), node }, &mut out);
                }
            }
            for index in messages {
                if let Some(row) = db.get("messages").and_then(Value::as_list).and_then(|rows| rows.get(index)) {
                    if let Some(node) = views::message_node(row) {
                        self.work.content_builds += 1;
                        self.emit(ViewOp::Replace { id: node.id.clone(), node }, &mut out);
                    }
                }
            }
            for owner in dirty {
                if let Some(index) = owner.strip_prefix("plugin:").and_then(|index| index.parse::<usize>().ok()) {
                    let section = &sections[index];
                    if self.changed(&owner, db, &section.inputs) { self.slot(&format!("plugin.{}", section.plugin), Some(views::section_node(section, db)), &mut out); }
                    continue;
                }
                let inputs: &[&str] = match owner.as_str() {
                    "header" => &["session.provider", "session.model", "session.status", "session.turn", "attempts"],
                    "turn" => &["session.status"], "panel" => &["panel"], "notices" => &["notices"],
                    "queue" => &["session.queue"], "attachments" => &["session.attachments"], _ => unreachable!(),
                };
                let paths = inputs.iter().map(|path| Path::parse(path).unwrap()).collect::<Vec<_>>();
                if !self.changed(&owner, db, &paths) { continue; }
                let node = match owner.as_str() {
                    "header" => Some(views::header_with_totals(db.get("session"), self.spend, self.context)),
                    "turn" => views::cancel(db), "panel" => views::panel(db), "notices" => views::notices(db),
                    "queue" => views::queue(db), "attachments" => views::attachments(db), _ => unreachable!(),
                };
                self.slot(&owner, node, &mut out);
            }
        }
        let previous = self.version.clone();
        self.version.rev = rev;
        self.history.push_back(Change { from: previous, version: self.version.clone(), ops: out });
        while self.history.len() > RETAINED_REVISIONS { self.history.pop_front(); }
        // Rebuild remains the specification, never the production update mechanism.
        if cfg!(test) || (cfg!(debug_assertions) && self.tree.len() <= 128) {
            assert_eq!(self.tree.snapshot(), views::document(db, sections), "incremental view differs from database rebuild");
        }
    }

    pub fn sync(&mut self, since: Option<&Version>, streams: Vec<misa_proto::sync::Stream>) -> ViewSync {
        if let Some(since) = since {
            if since == &self.version { return ViewSync::Changes { version: self.version.clone(), changes: vec![], streams }; }
            if let Some(at) = self.history.iter().position(|change| &change.from == since) {
                return ViewSync::Changes { version: self.version.clone(), changes: self.history.iter().skip(at).cloned().collect(), streams };
            }
        }
        self.work.snapshot_nodes += self.tree.len() as u64;
        ViewSync::Snapshot { version: self.version.clone(), view: self.tree.snapshot(), streams }
    }
}

fn overlaps(a: &Path, b: &Path) -> bool { a.segments().starts_with(b.segments()) || b.segments().starts_with(a.segments()) }
fn cost(row: &Value) -> i64 { row.get("cost_micros").and_then(Value::as_i64).unwrap_or(0) }
fn tokens(row: &Value) -> i64 { row.get("input_tokens").and_then(Value::as_i64).unwrap_or(0) }

#[cfg(test)]
mod tests {
    use super::*;
    fn advance(view: &mut Canonical, db: &mut Value, patches: Vec<(Path, Op)>, sections: &[Section]) {
        let after = misa_value::apply(db, &patches).unwrap();
        let change = DbChange { before: db.clone(), after: after.clone(), patches };
        *db = after;
        view.advance(db, &[change], sections, view.version.rev + 1);
    }
    fn patch(path: &str, op: Op) -> (Path, Op) { (Path::parse(path).unwrap(), op) }
    fn row(id: i64) -> Value { Value::map([("id", Value::Int(id)), ("text", Value::str("queued")), ("attachments", Value::list([]))]) }
    #[test]
    fn sequential_queue_deletes_keep_the_surviving_identity() {
        let mut db = views::initial_state("test", "p", "m", 0);
        let mut view = Canonical::new(&db, &[], "epoch".into());
        advance(&mut view, &mut db, vec![patch("session.queue", Op::Append(row(1))), patch("session.queue", Op::Append(row(2))), patch("session.queue", Op::Append(row(3)))], &[]);
        advance(&mut view, &mut db, vec![patch("session.queue[0]", Op::Delete), patch("session.queue[0]", Op::Delete)], &[]);
        assert_eq!(view.tree.children("queue"), vec!["queue.3"]);
    }
    #[test]
    fn retention_catches_up_atomically_or_sends_the_current_snapshot() {
        let mut db = views::initial_state("test", "p", "m", 0);
        let mut view = Canonical::new(&db, &[], "epoch".into());
        let original = view.version.clone();
        for n in 0..64 { advance(&mut view, &mut db, vec![patch("session.model", Op::Set(Value::str(format!("m{n}"))))], &[]); }
        match view.sync(Some(&original), vec![]) {
            ViewSync::Changes { changes, version, .. } => { assert_eq!(changes.len(), 64); assert_eq!(version.rev, 64); assert_eq!(changes[0].from, original); }
            _ => panic!("retained history must catch up"),
        }
        advance(&mut view, &mut db, vec![patch("session.model", Op::Set(Value::str("last")))], &[]);
        for since in [original, Version { epoch: "previous process".into(), rev: 65 }] {
            assert!(matches!(view.sync(Some(&since), vec![]), ViewSync::Snapshot { version: Version {rev:65,..}, .. }));
        }
        let current = view.version.clone();
        assert!(matches!(view.sync(Some(&current), vec![]), ViewSync::Changes {changes,..} if changes.is_empty()));
    }
    #[test]
    fn unrelated_patches_do_not_rebuild_declared_plugin_content() {
        let builds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = builds.clone();
        let sections = [Section { plugin: "counter".into(), inputs: vec![Path::parse("session.model").unwrap()], build: std::sync::Arc::new(move |_| { counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed); Ok(Node::section("counter").id("counter")) }) }];
        let mut db = views::initial_state("test", "p", "m", 0);
        let mut view = Canonical::new(&db, &sections, "epoch".into());
        let work = view.work.clone();
        advance(&mut view, &mut db, vec![patch("session.queue_seq", Op::Set(Value::Int(1)))], &sections);
        assert_eq!(view.work, work);
        // The test-only differential oracle invokes the rebuild once; production did not.
        assert_eq!(builds.load(std::sync::atomic::Ordering::Relaxed), 2);
    }
}
