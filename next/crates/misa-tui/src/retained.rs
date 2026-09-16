//! Retained line owners. Canonical operations format only their affected owner;
//! stream appends visit appended characters, then the visible viewport.
use std::collections::HashMap;
use misa_proto::{Node, SessionMsg, SessionEvent};
use misa_proto::view::Kind;
use misa_proto::sync::{IndexedTree, ViewOp, Stream, StreamUpdate};
use misa_render::Line;
use crate::Screen;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work { pub formatted_nodes: usize, pub appended_bytes: usize, pub copied_rows: usize, pub index_steps: usize }
struct Owner { lines: Vec<Line>, members: Vec<String>, depth: usize, level: usize, branch: bool, footer: bool }
struct Live { stream: Stream, lines: Vec<Line>, column: usize, last_nonblank: usize }
#[derive(Clone)]
enum Segment { Owner(String), Live(String) }
/// Prefix sums support viewport lookup and stream growth in logarithmic work.
#[derive(Default)]
struct Rows { sums: Vec<usize> }
impl Rows {
    fn new(lengths: &[usize]) -> Self {
        let mut rows = Self { sums: vec![0; lengths.len() + 1] };
        for (i, &len) in lengths.iter().enumerate() { rows.change(i, 0, len); }
        rows
    }
    fn change(&mut self, index: usize, old: usize, new: usize) -> usize {
        let mut i = index + 1;
        let mut steps = 0;
        while i < self.sums.len() { self.sums[i] = self.sums[i] - old + new; i += i & i.wrapping_neg(); steps += 1; }
        steps
    }
    fn prefix(&self, mut end: usize) -> usize {
        let mut sum = 0;
        while end > 0 { sum += self.sums[end]; end &= end - 1; }
        sum
    }
    fn total(&self) -> usize { self.prefix(self.sums.len().saturating_sub(1)) }
    fn locate(&self, row: usize) -> (usize, usize, usize) {
        let (mut index, mut sum, mut steps) = (0, 0, 0);
        let mut bit = self.sums.len().next_power_of_two() / 2;
        while bit > 0 {
            let next = index + bit;
            if next < self.sums.len() && sum + self.sums[next] <= row { index = next; sum += self.sums[next]; }
            bit /= 2; steps += 1;
        }
        (index, row.saturating_sub(sum), steps)
    }
}
pub struct Retained {
    tree: IndexedTree,
    owners: HashMap<String, Owner>,
    membership: HashMap<String, String>,
    order: Vec<String>,
    live: HashMap<String, Live>,
    segments: Vec<Segment>,
    live_positions: HashMap<String, usize>,
    rows: Rows,
    lengths: Vec<usize>,
    attachments: usize,
    panel: Option<String>,
    selection_body: Option<crate::select::Body>,
    width: u16,
    theme: String,
    opened: Vec<String>,
    components: misa_render::components::Settings,
    pub work: Work,
}
impl Retained {
    pub fn new(view: Node, screen: &Screen) -> Self {
        let tree = IndexedTree::new(view);
        let view = tree.snapshot();
        let mut out = Self { tree, owners: HashMap::new(), membership: HashMap::new(), order: vec![], live: HashMap::new(), segments: vec![], live_positions: HashMap::new(), rows: Rows::default(), lengths: vec![], attachments: 0, panel: None, selection_body: None, width: screen.width, theme: screen.theme.name.clone(), opened: screen.prefs.opened.clone(), components: screen.prefs.components.clone(), work: Work::default() };
        out.order = out.build(view, 0, 0, screen);
        out.reindex();
        out
    }
    pub fn interaction(&self) -> Node {
        let mut tree = self.tree.snapshot();
        let mut overlay = Node::section("streams").id("streams");
        let mut streams: Vec<_> = self.live.values().collect(); streams.sort_by_key(|live| &live.stream.id);
        for live in streams {
            let stream = &live.stream;
            if self.tree.contains(stream.id.rsplit_once('.').map(|(owner, _)| owner).unwrap_or(&stream.id)) { continue; }
            overlay.children.push(Node::text(&stream.role, [misa_proto::view::Span::plain(&stream.text)]).id(&stream.id));
        }
        if !overlay.children.is_empty() {
            if let Some(transcript) = tree.children.iter_mut().find(|n| n.id == "transcript") { transcript.children.push(overlay); }
            else { tree.children.push(overlay); }
        }
        tree
    }
    pub fn has_turn(&self) -> bool { self.tree.contains("turn") }
    pub fn selection_key(&mut self, screen: &mut Screen, key: &crate::Key) -> Option<crate::KeyOut> {
        if !screen.reading_key(key) { return None; }
        // A key can extend the selection into text that arrived since its last
        // movement. Passive stream repaint reuses the prior selection body.
        self.selection_body = None;
        self.cache_selection();
        screen.selection_in(self.selection_body.as_ref().unwrap(), key)
    }
    fn cache_selection(&mut self) {
        if self.selection_body.is_none() {
            let all: Vec<_> = self.segments.iter().enumerate().flat_map(|(i, segment)| self.lines(segment)[..self.lengths[i]].iter().cloned()).collect();
            self.selection_body = Some(crate::select::Body::of(&all));
        }
    }
    fn build(&mut self, mut node: Node, depth: usize, level: usize, screen: &Screen) -> Vec<String> {
        let id = node.id.clone();
        let context = misa_render::components::Context { theme: &screen.theme, columns: screen.width as usize, settings: &screen.prefs.components, values: &screen.values };
        let component = screen.components.render(&screen.local_presentation.model(&node, screen), &context);
        let footer = screen.components.placement(&node.role, &screen.prefs.components) == misa_render::components::Placement::Footer;
        let branch = component.is_none() && (matches!(node.kind, Kind::Section) || matches!(node.kind, Kind::Collapsible { .. }) && screen.prefs.is_open(&id));
        let children = if branch { std::mem::take(&mut node.children) } else { vec![] };
        let resolved = screen.resolve(&node);
        let mut members = vec![];
        fn ids(node: &Node, members: &mut Vec<String>, attachments: &mut usize) {
            members.push(node.id.clone());
            *attachments += usize::from(node.actions.iter().any(|a| a.id == "attachment.save"));
            for child in &node.children { ids(child, members, attachments); }
        }
        ids(&node, &mut members, &mut self.attachments);
        self.work.formatted_nodes += members.len();
        for member in &members { self.membership.insert(member.clone(), id.clone()); }
        let child_depth = depth + usize::from(screen.theme.rail(&node.role).is_some());
        let lines = component.unwrap_or_else(|| misa_render::lines::render_block(&resolved, &screen.theme, screen.width as usize, depth));
        self.owners.insert(id.clone(), Owner { lines, members, depth, level, branch, footer });
        let mut order = vec![id];
        for child in children { order.extend(self.build(child, child_depth, level + 1, screen)); }
        order
    }
    fn range(&self, id: &str) -> std::ops::Range<usize> {
        let start = self.order.iter().position(|candidate| candidate == id).expect("layout owner");
        let level = self.owners[id].level;
        let end = (start + 1..self.order.len()).find(|&i| self.owners[&self.order[i]].level <= level).unwrap_or(self.order.len());
        start..end
    }
    fn erase(&mut self, range: std::ops::Range<usize>) {
        for id in self.order.drain(range) {
            let owner = self.owners.remove(&id).unwrap();
            for member in owner.members { self.membership.remove(&member); }
        }
    }
    fn refresh(&mut self, id: &str, screen: &Screen) {
        let range = self.range(id);
        let at = range.start;
        let owner = &self.owners[id];
        let (depth, level) = (owner.depth, owner.level);
        self.erase(range);
        if let Some(node) = self.tree.subtree(id) {
            let added = self.build(node, depth, level, screen);
            self.order.splice(at..at, added);
        }
    }
    fn op(&mut self, op: &ViewOp, screen: &Screen) -> Result<(), String> {
        let target = match op { ViewOp::Insert { parent, .. } => parent, ViewOp::Remove { id } | ViewOp::Replace { id, .. } => id };
        let owner = self.membership.get(target).cloned().ok_or("layout target missing")?;
        let nested = &owner != target || matches!(op, ViewOp::Insert { .. }) && !self.owners[&owner].branch;
        if nested { self.tree.apply(op)?; self.refresh(&owner, screen); return Ok(()); }
        match op {
            ViewOp::Insert { parent, before, node } => {
                let parent_owner = &self.owners[parent];
                let (depth, level) = (parent_owner.depth + usize::from(screen.theme.rail(&self.tree.node(parent).unwrap().role).is_some()), parent_owner.level + 1);
                let at = before.as_ref().map(|id| self.range(id).start).unwrap_or_else(|| self.range(parent).end);
                self.tree.apply(op)?;
                let added = self.build(node.clone(), depth, level, screen);
                self.order.splice(at..at, added);
            }
            ViewOp::Remove { id } => { let range = self.range(id); self.tree.apply(op)?; self.erase(range); }
            ViewOp::Replace { id, .. } => { self.tree.apply(op)?; self.refresh(id, screen); }
        }
        Ok(())
    }
    fn reindex(&mut self) {
        self.segments.clear(); self.live_positions.clear();
        self.selection_body = None;
        self.panel = self.membership.keys().find(|id| self.tree.node(id).is_some_and(|node| node.role == "panel")).cloned();
        self.attachments = self.owners.values().flat_map(|o| &o.members).filter_map(|id| self.tree.node(id)).filter(|n| n.actions.iter().any(|a| a.id == "attachment.save")).count();
        let insert = if self.owners.get("transcript").is_some_and(|owner| owner.branch) { self.range("transcript").end } else { self.order.len() };
        let mut streams: Vec<_> = self.live.keys().filter(|id| !self.tree.contains(id.rsplit_once('.').map(|(owner, _)| owner).unwrap_or(id))).cloned().collect();
        streams.sort();
        for i in 0..=self.order.len() {
            if i == insert { for id in &streams { self.live_positions.insert(id.clone(), self.segments.len()); self.segments.push(Segment::Live(id.clone())); } }
            if let Some(id) = self.order.get(i) && !self.owners[id].footer && !self.owners[id].lines.is_empty() { self.segments.push(Segment::Owner(id.clone())); }
        }
        self.lengths = self.segments.iter().map(|segment| self.lines(segment).len()).collect();
        for index in (0..self.segments.len()).rev() {
            let rows = self.lines(&self.segments[index]);
            let trimmed = rows.iter().rposition(|line| !line.is_blank()).map_or(0, |at| at + 1);
            self.lengths[index] = trimmed;
            if trimmed > 0 { break; }
        }
        self.rows = Rows::new(&self.lengths);
    }
    fn lines(&self, segment: &Segment) -> &[Line] { match segment { Segment::Owner(id) => &self.owners[id].lines, Segment::Live(id) => &self.live[id].lines } }
    fn current(&mut self, stream: Stream, screen: &Screen) {
        let id = stream.id.clone();
        let mut live = Live { stream: Stream { text: String::new(), ..stream.clone() }, lines: vec![], column: 0, last_nonblank: 0 };
        live.append(&stream.text, screen);
        self.live.insert(id, live);
    }
    pub fn receive(&mut self, message: &SessionMsg, screen: &Screen) -> Result<(), String> {
        match message {
            SessionMsg::View { view, .. } => { *self = Self::new(view.clone(), screen); }
            SessionMsg::Changes { changes, .. } => { for change in changes { for op in &change.ops { self.op(op, screen)?; } } self.reindex(); }
            SessionMsg::Streams { streams } => { self.live.clear(); for stream in streams { self.current(stream.clone(), screen); } self.reindex(); }
            SessionMsg::Event { event: SessionEvent::Stream { update }, .. } => match update {
                StreamUpdate::Current { stream } => { self.current(stream.clone(), screen); self.reindex(); }
                StreamUpdate::End { id } => { self.live.remove(id); self.reindex(); }
                StreamUpdate::Append { id, offset, text } => {
                    let live = self.live.get_mut(id).ok_or("missing stream")?;
                    if live.stream.text.len() != *offset { return Err("stream offset gap".into()); }
                    live.append(text, screen);
                    self.work.appended_bytes += text.len();
                    if let Some(&index) = self.live_positions.get(id) {
                        let new = if self.rows.total() == self.rows.prefix(index + 1) { live.last_nonblank } else { live.lines.len() };
                        self.work.index_steps += self.rows.change(index, self.lengths[index], new);
                        self.lengths[index] = new;
                    }
                }
            },
            _ => {}
        }
        Ok(())
    }
    /// Apply a complete shared-client document transaction to derived render
    /// caches. Protocol cursors and recovery remain owned by the shared replica.
    pub fn observed(&mut self, update: &misa_client::document::Update, screen: &Screen) -> Result<(), String> {
        use misa_client::document::Update;
        use misa_protocol::observation::{Applied, MemberChange};
        match update {
            Update::Reset(document) => {
                *self = Self::new(document.tree.clone(), screen);
                for stream in &document.streams { self.current(stream.clone(), screen); }
                self.reindex();
            }
            Update::Changed { member, applied } => {
                let Applied::Changed(members) = applied.as_ref() else { return Err("Expected document transaction".into()); };
                let Some(MemberChange::Document { tree, live, reset_live }) = members.get(member) else { return Err("Expected document member changes".into()); };
                let mut structural = !tree.is_empty() || *reset_live;
                for op in tree { self.op(op, screen)?; }
                if *reset_live { self.live.clear(); }
                for update in live {
                    match update {
                        StreamUpdate::Current { stream } => { self.current(stream.clone(), screen); structural = true; }
                        StreamUpdate::End { id } => { self.live.remove(id); structural = true; }
                        StreamUpdate::Append { id, offset, text } => {
                            let live = self.live.get_mut(id).ok_or("Missing rendered stream")?;
                            if live.stream.text.len() != *offset { return Err("Rendered stream offset gap".into()); }
                            live.append(text, screen);
                            self.work.appended_bytes += text.len();
                            if !structural {
                                if let Some(&index) = self.live_positions.get(id) {
                                    let new = if self.rows.total() == self.rows.prefix(index + 1) { live.last_nonblank } else { live.lines.len() };
                                    self.work.index_steps += self.rows.change(index, self.lengths[index], new);
                                    self.lengths[index] = new;
                                }
                            }
                        }
                    }
                }
                if structural { self.reindex(); }
            }
            Update::Unavailable(_) | Update::Status(_) => {},
        }
        Ok(())
    }
    pub fn local(&mut self, screen: &Screen) {
        if self.width != screen.width || self.theme != screen.theme.name || self.opened != screen.prefs.opened || self.components != screen.prefs.components {
            let streams: Vec<_> = self.live.values().map(|live| live.stream.clone()).collect();
            *self = Self::new(self.tree.snapshot(), screen);
            for stream in streams { self.current(stream, screen); }
            self.reindex();
        } else if let Some(panel) = self.panel.clone() {
            if let Some(id) = self.membership.get(&panel).cloned() { self.refresh(&id, screen); self.reindex(); }
        }
        self.selection_body = None;
    }
    #[cfg(test)]
    pub fn draw(&mut self, screen: &Screen) -> Vec<Line> {
        self.frame(screen, None).lines
    }
    #[cfg(test)]
    pub fn frame(&mut self, screen: &Screen, staging: Option<&str>) -> crate::chrome::Frame {
        self.frame_with(screen, staging, &[], &[])
    }
    /// Independent documents share placement and components, never node identity.
    pub fn placed_lines(&self, limit: usize) -> (Vec<Line>, Vec<Line>) {
        let document = self.segments.iter().enumerate().flat_map(|(index, segment)| self.lines(segment)[..self.lengths[index]].iter().cloned()).take(limit).collect();
        let footer = self.order.iter().filter_map(|id| self.owners.get(id)).filter(|owner| owner.footer).flat_map(|owner| owner.lines.iter().cloned()).take(limit).collect();
        (document, footer)
    }
    pub fn frame_with(&mut self, screen: &Screen, staging: Option<&str>, extra_document: &[Line], extra_footer: &[Line]) -> crate::chrome::Frame {
        let chrome = crate::chrome::frame(screen, self.attachments, staging);
        let available = (screen.height as usize).saturating_sub(chrome.lines.len());
        let footer: Vec<_> = self.order.iter().filter_map(|id| self.owners.get(id)).filter(|owner| owner.footer)
            .flat_map(|owner| owner.lines.iter().cloned()).take(available).collect();
        let remaining = available.saturating_sub(footer.len());
        let extra_footer = &extra_footer[..extra_footer.len().min(remaining)];
        let extra_document = &extra_document[..extra_document.len().min(remaining.saturating_sub(extra_footer.len()))];
        let room = remaining.saturating_sub(extra_footer.len() + extra_document.len());
        let total = self.rows.total();
        let start = if screen.follow { total.saturating_sub(room) } else { screen.scroll.min(total.saturating_sub(1)) };
        let (mut segment, mut offset, steps) = self.rows.locate(start);
        self.work.index_steps += steps;
        let mut lines = vec![];
        while lines.len() < room && segment < self.segments.len() {
            let rows = &self.lines(&self.segments[segment])[..self.lengths[segment]];
            let take = (room - lines.len()).min(rows.len().saturating_sub(offset));
            lines.extend_from_slice(&rows[offset..offset + take]);
            segment += 1; offset = 0;
        }
        self.work.copied_rows += lines.len();
        if let Some(selection) = &screen.selection {
            // Selection is explicit local work. It uses cached rows, never tree formatting.
            if self.selection_body.is_none() {
                let all: Vec<_> = self.segments.iter().enumerate().flat_map(|(i, segment)| self.lines(segment)[..self.lengths[i]].iter().cloned()).collect();
                self.selection_body = Some(crate::select::Body::of(&all));
            }
            for (i, line) in lines.iter_mut().enumerate() { if let Some((from, to)) = selection.on_row(self.selection_body.as_ref().unwrap(), start + i) { crate::select_highlight(line, from, to, &screen.theme); } }
        }
        lines.extend_from_slice(extra_document);
        lines.extend(footer);
        lines.extend_from_slice(extra_footer);
        let cursor_row = lines.len() + chrome.cursor_row;
        lines.extend(chrome.lines);
        crate::chrome::Frame { lines, cursor_row, cursor_column: chrome.cursor_column }
    }
}
impl Live {
    fn append(&mut self, text: &str, screen: &Screen) {
        let width = (screen.width as usize).max(1);
        let style = screen.theme.role(&self.stream.role);
        for character in text.chars() {
            let cells = misa_render::width(&character.to_string());
            if self.lines.is_empty() || character != '\n' && self.column + cells > width && self.column > 0 {
                self.lines.push(Line { indent: 0, node: Some(self.stream.id.clone()), spans: vec![(style, String::new())] }); self.column = 0;
            }
            if character == '\n' { self.lines.push(Line { indent: 0, node: Some(self.stream.id.clone()), spans: vec![(style, String::new())] }); self.column = 0; }
            else { self.lines.last_mut().unwrap().spans[0].1.push(character); self.column += cells; if !character.is_whitespace() { self.last_nonblank = self.lines.len(); } }
        }
        self.stream.text.push_str(text);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn independent_documents_keep_their_own_node_identities_and_placement() {
        let screen = crate::Screen::new(80, 20);
        let make = |text: &str| misa_proto::Node::text("notice", [misa_proto::view::Span::plain(text)]).id("same");
        let mut left = super::Retained::new(make("left"), &screen);
        let right = super::Retained::new(make("right"), &screen);
        left.op(&misa_proto::sync::ViewOp::Replace { id: "same".into(), node: make("changed") }, &screen).unwrap();
        left.reindex();
        assert!(left.placed_lines(20).0.iter().any(|line| line.text().contains("changed")));
        assert!(right.placed_lines(20).0.iter().any(|line| line.text().contains("right")));
        let mut main = super::Retained::new(make("conversation"), &screen);
        let extras = left.placed_lines(20).0.into_iter().chain(right.placed_lines(20).0).collect::<Vec<_>>();
        let frame = main.frame_with(&screen, None, &extras, &[]);
        assert!(frame.lines.iter().any(|line| line.text().contains("conversation")));
        assert!(frame.lines.iter().any(|line| line.text().contains("changed")));
        assert!(frame.lines.iter().any(|line| line.text().contains("right")));
    }
    use super::*;
    use misa_proto::view::Span;
    fn text(id: &str, value: &str) -> Node { Node::text("assistant", [Span::plain(value)]).id(id) }
    fn document(history: usize) -> Node {
        let mut transcript = Node::section("transcript").id("transcript");
        for i in 0..history { transcript.children.push(Node::section("message").id(format!("msg{i}")).child(text(&format!("msg{i}.body"), "settled content"))); }
        Node::section("session").id("session").child(text("header", "Header")).child(transcript)
    }
    fn all(retained: &Retained) -> Vec<Line> {
        let mut lines: Vec<_> = retained.segments.iter().flat_map(|segment| retained.lines(segment).iter().cloned()).collect();
        while lines.last().is_some_and(Line::is_blank) { lines.pop(); }
        lines
    }
    fn oracle(retained: &Retained, screen: &Screen) {
        assert_eq!(all(retained), misa_render::render(&screen.resolve(&retained.tree.snapshot()), &screen.theme, screen.width as usize));
    }
    #[test]
    fn shared_document_updates_keep_token_work_incremental_and_settle_atomically() {
        use misa_client::document::Update;
        use misa_protocol::observation::{Applied, MemberChange};
        let update = |tree, live| Update::Changed { member: "body".into(), applied: std::sync::Arc::new(Applied::Changed(std::collections::BTreeMap::from([
            ("body".into(), MemberChange::Document { tree, live, reset_live: false }),
        ]))) };
        for history in [100, 1000] {
            let screen = Screen::new(40, 16);
            let mut retained = Retained::new(document(history), &screen);
            retained.current(Stream { id: "new.body".into(), role: "assistant".into(), text: String::new() }, &screen);
            retained.reindex();
            retained.work = Work::default();
            retained.observed(&update(vec![], vec![StreamUpdate::Append { id: "new.body".into(), offset: 0, text: "é🙂".into() }]), &screen).unwrap();
            assert_eq!(retained.work.formatted_nodes, 0);
            assert_eq!(retained.work.appended_bytes, 6);
            assert!(retained.work.index_steps <= 24);
            retained.observed(&update(vec![ViewOp::Insert { parent: "transcript".into(), before: None, node: text("new", "é🙂") }],
                vec![StreamUpdate::End { id: "new.body".into() }]), &screen).unwrap();
            assert!(retained.live.is_empty());
            assert!(retained.tree.contains("new"));
            oracle(&retained, &screen);
        }
    }
    #[test]
    fn canonical_operations_match_full_render_after_every_operation() {
        for _ in 0..6 {
            let mut screen = Screen::new(37, 24);
            let mut retained = Retained::new(document(20), &screen);
            oracle(&retained, &screen);
            let operations = [
                ViewOp::Insert { parent: "transcript".into(), before: Some("msg4".into()), node: text("inserted", "inserted at a stable anchor") },
                ViewOp::Replace { id: "msg3.body".into(), node: text("msg3.body", "a much longer replacement that wraps into multiple lines at this width") },
                ViewOp::Remove { id: "msg7".into() },
                ViewOp::Insert { parent: "msg4".into(), before: None, node: text("msg4.extra", "another body") },
                ViewOp::Replace { id: "msg4".into(), node: text("msg4", "section becomes content") },
            ];
            for op in operations { retained.op(&op, &screen).unwrap(); retained.reindex(); oracle(&retained, &screen); }
            screen.width = 19; retained.local(&screen); oracle(&retained, &screen);
        }
    }
    #[test]
    fn append_formatting_and_viewport_copy_work_do_not_grow_with_history() {
        let mut results = vec![];
        let mut baselines = vec![];
        for history in [100, 1000] {
            for _ in 0..6 {
                let screen = Screen::new(40, 16);
                let mut retained = Retained::new(document(history), &screen);
                retained.current(Stream { id: "msg9999.body".into(), role: "assistant".into(), text: "current ".into() }, &screen);
                retained.reindex(); retained.work = Work::default();
                let mut offset = "current ".len();
                let mut baseline_visits = 0;
                let mut baseline_output = String::new();
                for text in ["one ", "two ", "界🙂", "\n", "final"] {
                    retained.receive(&SessionMsg::Event { seq: 1, event: SessionEvent::Stream { update: StreamUpdate::Append { id: "msg9999.body".into(), offset, text: text.into() } } }, &screen).unwrap();
                    offset += text.len();
                    let rendered = retained.draw(&screen);
                    assert!(rendered.iter().any(|line| line.node.as_deref() == Some("msg9999.body")));
                    // The previous full-tree presentation path remains callable as
                    // a baseline. Count its actual recursive resolve visits.
                    let legacy = retained.interaction();
                    crate::RESOLVE_VISITS.with(|visits| visits.set(0));
                    baseline_output.push_str(&misa_render::to_plain(&crate::draw(&screen, &legacy)));
                    baseline_visits += crate::RESOLVE_VISITS.with(|visits| visits.get());
                }
                assert_eq!(retained.live["msg9999.body"].stream.text, "current one two 界🙂\nfinal");
                assert_eq!(retained.work.formatted_nodes, 0);
                assert_eq!(retained.work.appended_bytes, 21);
                results.push(retained.work);
                baselines.push((baseline_visits, baseline_output));
            }
        }
        assert!(results.iter().all(|work| (work.formatted_nodes, work.appended_bytes, work.copied_rows) == (results[0].formatted_nodes, results[0].appended_bytes, results[0].copied_rows)), "{results:?}");
        assert!(results.iter().all(|work| work.index_steps <= 5 * 24));
        assert!(baselines[..6].iter().all(|run| run == &baselines[0]));
        assert!(baselines[6..].iter().all(|run| run == &baselines[6]));
        eprintln!("baseline actual resolve visits: 100-history {}, 1000-history {}; one output per size across six runs", baselines[0].0, baselines[6].0);
        eprintln!("100/1000 history, six runs each: {results:?}");
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::{Key, KeyOut};
    use misa_proto::view::Span;
    #[test]
    fn live_selection_copies_the_displayed_character_wrapped_row() {
        let mut screen = Screen::new(10, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(Stream { id: "msg1.body".into(), role: "assistant".into(), text: "hello world".into() }, &screen); retained.reindex();
        assert_eq!(retained.live["msg1.body"].lines.iter().map(Line::text).collect::<Vec<_>>(), ["hello worl", "d"]);
        screen.key(Key::Escape);
        assert_eq!(retained.selection_key(&mut screen, &Key::Char('v')), Some(KeyOut::Local));
        retained.selection_key(&mut screen, &Key::Motion(crate::ed::Motion::LineEnd));
        assert_eq!(retained.selection_key(&mut screen, &Key::Char('y')), Some(KeyOut::Copy("d".into())));
    }
    #[test]
    fn full_viewport_reserves_staged_attachment_and_multiline_editor_rows() {
        let mut root = Node::section("session").id("session");
        for i in 0..100 { root.children.push(Node::text("text", [Span::plain("history")]).id(format!("n{i}"))); }
        let mut screen = Screen::new(30, 12); screen.editor.set_text("first\nsecond");
        let mut retained = Retained::new(root, &screen);
        let frame = retained.frame(&screen, Some("1 clipboard attachments · 1 uploading"));
        assert_eq!(frame.lines.len(), 12);
        assert!(frame.lines.iter().all(|line| !line.text().contains('\n') && misa_render::width(&line.text()) <= 30));
        assert_eq!(frame.cursor_row, 11); assert_eq!(frame.cursor_column, 8);
    }
    #[test]
    fn folded_content_edits_and_trailing_blanks_match_the_settled_renderer() {
        let mut screen = Screen::new(40, 40);
        let folded = Node::new("call", Kind::Collapsible { summary: vec![Span::plain("summary")] }).id("call")
            .child(Node::new("quote", Kind::Quote).id("quote").child(Node::text("text", [Span::plain("before")]).id("body")));
        let root = Node::section("session").id("session").child(folded).child(Node::text("text", [Span::plain("")]).id("blank"));
        let mut retained = Retained::new(root, &screen);
        retained.op(&ViewOp::Replace { id: "body".into(), node: Node::text("text", [Span::plain("after")]).id("body") }, &screen).unwrap(); retained.reindex();
        screen.prefs.open_all(); retained.local(&screen);
        let expected = misa_render::render(&screen.resolve(&retained.tree.snapshot()), &screen.theme, 40);
        let frame = retained.frame(&screen, None);
        assert_eq!(&frame.lines[..expected.len()], expected.as_slice());
        assert_eq!(frame.lines.len(), expected.len() + crate::chrome::frame(&screen, 0, None).lines.len());
    }
}
