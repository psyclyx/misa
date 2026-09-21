//! Retained line owners. Canonical operations format only their affected owner;
//! stream appends visit appended characters, then the visible viewport.
use crate::Screen;
use misa_lines::Line;
use misa_proto::Node;
use misa_proto::sync::{IndexedTree, Stream, StreamUpdate, ViewOp};
use misa_proto::view::{ActionOn, Kind};
use misa_render::{Style, Theme};
use std::collections::HashMap;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub formatted_nodes: usize,
    pub appended_bytes: usize,
    pub copied_rows: usize,
    pub index_steps: usize,
}
struct Owner {
    lines: Vec<Line>,
    members: Vec<String>,
    role: String,
    depth: usize,
    level: usize,
    branch: bool,
    footer: bool,
}
struct Live {
    stream: Stream,
    lines: Vec<Line>,
    /// The parsed markdown of the in-flight text, reused block by block as the
    /// stream grows, so streaming renders through the same path as a settled body.
    document: Option<misa_markdown::Document>,
    /// Each block's laid-out lines, so only the blocks an append changed are
    /// rendered again. Invalidated by a width change.
    rendered: Vec<(misa_proto::Node, Vec<Line>)>,
    rendered_width: usize,
    last_nonblank: usize,
    /// Some(limit) while a collapsed thinking stream follows its own tail: the
    /// reader is watching the current reasoning, not the beginning of a summary
    /// that has not been written yet. An opened stream keeps every line.
    tail: Option<usize>,
}

/// How many trailing lines of a collapsed thinking stream stay visible while it
/// grows. The settled block replaces them with its head preview.
const THINKING_TAIL_LINES: usize = 3;

/// Whether a stream role belongs to the model reasoning rather than to its answer.
fn thinking_stream(role: &str) -> bool {
    role.starts_with("message.assistant.thinking") || role.starts_with("message.thinking")
}

#[derive(Clone)]
enum Segment {
    Owner(String),
    Live(String),
}

/// The order two live streams of one pending message render in.
///
/// A message has a thinking stream and a text stream, and the answer must not
/// appear above the reasoning that produced it. The suffix decides that, not the
/// stream id's alphabetical order (`text` sorts before `thinking`).
fn stream_order(id: &str) -> (String, u8) {
    let (owner, suffix) = id.rsplit_once('.').unwrap_or((id, ""));
    let rank = match suffix {
        "thinking" => 0,
        "text" => 1,
        _ => 2,
    };
    (owner.to_string(), rank)
}

/// Prefix sums support viewport lookup and stream growth in logarithmic work.
#[derive(Default)]
struct Rows {
    sums: Vec<usize>,
}
impl Rows {
    fn new(lengths: &[usize]) -> Self {
        let mut rows = Self {
            sums: vec![0; lengths.len() + 1],
        };
        for (i, &len) in lengths.iter().enumerate() {
            rows.change(i, 0, len);
        }
        rows
    }
    fn change(&mut self, index: usize, old: usize, new: usize) -> usize {
        let mut i = index + 1;
        let mut steps = 0;
        while i < self.sums.len() {
            self.sums[i] = self.sums[i] - old + new;
            i += i & i.wrapping_neg();
            steps += 1;
        }
        steps
    }
    fn prefix(&self, mut end: usize) -> usize {
        let mut sum = 0;
        while end > 0 {
            sum += self.sums[end];
            end &= end - 1;
        }
        sum
    }
    fn total(&self) -> usize {
        self.prefix(self.sums.len().saturating_sub(1))
    }
    fn locate(&self, row: usize) -> (usize, usize, usize) {
        let (mut index, mut sum, mut steps) = (0, 0, 0);
        let mut bit = self.sums.len().next_power_of_two() / 2;
        while bit > 0 {
            let next = index + bit;
            if next < self.sums.len() && sum + self.sums[next] <= row {
                index = next;
                sum += self.sums[next];
            }
            bit /= 2;
            steps += 1;
        }
        (index, row.saturating_sub(sum), steps)
    }
}
/// A semantic position, independent of the physical row it currently occupies.
///
/// The previous system anchored the viewport to a source node and a character
/// offset; the rewrite's rendered lines only carry the node id, so the anchor is
/// the node under the top row and its ordinal among the consecutive rows that
/// share it. That is enough to hold a reader's place while earlier content grows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Anchor {
    node: Option<String>,
    offset: usize,
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
    components: misa_lines::components::Settings,
    /// The semantic row the reader scrolled to, when not following.
    anchor: Option<Anchor>,
    /// The layout generation `anchor_row` was resolved against, so an ordinary
    /// repaint does not re-walk the document.
    anchor_epoch: u64,
    /// The resolved first row for the current anchor and layout.
    anchor_row: usize,
    /// The last `Screen::scroll_intent` this viewport resolved.
    last_intent: u64,
    /// The selection head the viewport last revealed, so a movement scrolls the
    /// caret into view once rather than fighting a reader who scrolled away.
    revealed_head: Option<crate::select::Spot>,
    /// Bumped whenever retained lengths change.
    layout_epoch: u64,
    /// The first row the last frame resolved to; the event loop writes it back to
    /// `Screen::scroll` so the next delta is relative to what was shown.
    resolved_scroll: usize,
    /// Whether the viewport is showing the tail. It mirrors `Screen::follow` and
    /// is also set when a reader scrolls to the last page, so new output keeps
    /// arriving in view once they catch up.
    following: bool,
    pub work: Work,
}
impl Retained {
    pub fn new(view: Node, screen: &Screen) -> Self {
        let tree = IndexedTree::new(view);
        let view = tree.snapshot();
        let mut out = Self {
            tree,
            owners: HashMap::new(),
            membership: HashMap::new(),
            order: vec![],
            live: HashMap::new(),
            segments: vec![],
            live_positions: HashMap::new(),
            rows: Rows::default(),
            lengths: vec![],
            attachments: 0,
            panel: None,
            selection_body: None,
            width: screen.width,
            theme: screen.theme.name.clone(),
            opened: screen.prefs.opened.clone(),
            components: screen.prefs.components.clone(),
            anchor: None,
            anchor_epoch: 0,
            anchor_row: 0,
            last_intent: 0,
            revealed_head: None,
            layout_epoch: 0,
            resolved_scroll: 0,
            following: screen.follow,
            work: Work::default(),
        };
        out.order = out.build(view, 0, 0, screen);
        out.reindex();
        out
    }
    pub fn interaction(&self) -> Node {
        let mut tree = self.tree.snapshot();
        let mut overlay = Node::section("streams").id("streams");
        let mut streams: Vec<_> = self.live.values().collect();
        streams.sort_by_key(|live| stream_order(&live.stream.id));
        for live in streams {
            let stream = &live.stream;
            if self.tree.contains(
                stream
                    .id
                    .rsplit_once('.')
                    .map(|(owner, _)| owner)
                    .unwrap_or(&stream.id),
            ) {
                continue;
            }
            overlay.children.push(
                Node::text(&stream.role, [misa_proto::view::Span::plain(&stream.text)])
                    .id(&stream.id),
            );
        }
        if !overlay.children.is_empty() {
            if let Some(transcript) = tree.children.iter_mut().find(|n| n.id == "transcript") {
                transcript.children.push(overlay);
            } else {
                tree.children.push(overlay);
            }
        }
        tree
    }
    pub fn has_turn(&self) -> bool {
        self.tree.contains("turn")
    }
    pub fn selection_key(
        &mut self,
        screen: &mut Screen,
        key: &crate::Key,
    ) -> Option<crate::KeyOut> {
        if !screen.reading_key(key) {
            return None;
        }
        // A key can extend the selection into text that arrived since its last
        // movement. Passive stream repaint reuses the prior selection body.
        self.selection_body = None;
        self.cache_selection();
        screen.selection_in(self.selection_body.as_ref().unwrap(), key)
    }
    fn cache_selection(&mut self) {
        if self.selection_body.is_none() {
            let all: Vec<_> = self
                .segments
                .iter()
                .enumerate()
                .flat_map(|(i, segment)| self.lines(segment)[..self.lengths[i]].iter().cloned())
                .collect();
            self.selection_body = Some(crate::select::Body::of(&all));
        }
    }
    fn build(
        &mut self,
        mut node: Node,
        depth: usize,
        level: usize,
        screen: &Screen,
    ) -> Vec<String> {
        let id = node.id.clone();
        let context = misa_lines::components::Context {
            theme: &screen.theme,
            columns: screen.width as usize,
            settings: &screen.prefs.components,
            values: &screen.values,
        };
        let component = screen
            .components
            .render(&screen.local_presentation.model(&node, screen), &context);
        let footer = screen
            .components
            .placement(&node.role, &screen.prefs.components)
            == misa_lines::components::Placement::Footer;
        // A section is a structural layout boundary. A message or collapsible,
        // however, is a complete semantic unit: keeping its children together
        // is what lets the shared renderer apply its rail, markdown wrapping,
        // and disclosure policy as one contract. Splitting those nodes into
        // anonymous owners loses exactly the context the old transcript used.
        let branch = component.is_none()
            && matches!(node.kind, Kind::Section)
            // Message sections are semantic render units, not layout containers.
            // Keeping their subtree in one owner is what gives the common renderer
            // the role needed to draw the transcript rail and markdown projection.
            && !node.role.starts_with("message.");
        let children = if branch {
            std::mem::take(&mut node.children)
        } else {
            vec![]
        };
        let resolved = screen.resolve(&node);
        let mut members = vec![];
        fn ids(node: &Node, members: &mut Vec<String>, attachments: &mut usize) {
            members.push(node.id.clone());
            *attachments += usize::from(node.actions.iter().any(|a| a.id == "attachment.save"));
            for child in &node.children {
                ids(child, members, attachments);
            }
        }
        ids(&node, &mut members, &mut self.attachments);
        self.work.formatted_nodes += members.len();
        for member in &members {
            self.membership.insert(member.clone(), id.clone());
        }
        let child_depth = depth + usize::from(screen.theme.rail(&node.role).is_some());
        let lines = component.unwrap_or_else(|| {
            misa_lines::render_block(&resolved, &screen.theme, screen.width as usize, depth)
        });
        self.owners.insert(
            id.clone(),
            Owner {
                lines,
                members,
                role: node.role.clone(),
                depth,
                level,
                branch,
                footer,
            },
        );
        let mut order = vec![id];
        for child in children {
            order.extend(self.build(child, child_depth, level + 1, screen));
        }
        order
    }
    fn range(&self, id: &str) -> std::ops::Range<usize> {
        let start = self
            .order
            .iter()
            .position(|candidate| candidate == id)
            .expect("layout owner");
        let level = self.owners[id].level;
        let end = (start + 1..self.order.len())
            .find(|&i| self.owners[&self.order[i]].level <= level)
            .unwrap_or(self.order.len());
        start..end
    }
    fn erase(&mut self, range: std::ops::Range<usize>) {
        for id in self.order.drain(range) {
            let owner = self.owners.remove(&id).unwrap();
            for member in owner.members {
                self.membership.remove(&member);
            }
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
        let target = match op {
            ViewOp::Insert { parent, .. } => parent,
            ViewOp::Remove { id } | ViewOp::Replace { id, .. } => id,
        };
        let owner = self
            .membership
            .get(target)
            .cloned()
            .ok_or("layout target missing")?;
        let nested =
            &owner != target || matches!(op, ViewOp::Insert { .. }) && !self.owners[&owner].branch;
        if nested {
            self.tree.apply(op)?;
            self.refresh(&owner, screen);
            return Ok(());
        }
        match op {
            ViewOp::Insert {
                parent,
                before,
                node,
            } => {
                let parent_owner = &self.owners[parent];
                let parent_node = self.tree.node(parent).unwrap();
                let (depth, level) = (
                    parent_owner.depth
                        + usize::from(screen.theme.rail(&parent_node.role).is_some()),
                    parent_owner.level + 1,
                );
                let at = before
                    .as_ref()
                    .map(|id| self.range(id).start)
                    .unwrap_or_else(|| self.range(parent).end);
                self.tree.apply(op)?;
                let added = self.build(node.clone(), depth, level, screen);
                self.order.splice(at..at, added);
            }
            ViewOp::Remove { id } => {
                let range = self.range(id);
                self.tree.apply(op)?;
                self.erase(range);
            }
            ViewOp::Replace { id, .. } => {
                self.tree.apply(op)?;
                self.refresh(id, screen);
            }
        }
        Ok(())
    }
    fn reindex(&mut self) {
        self.segments.clear();
        self.live_positions.clear();
        self.selection_body = None;
        self.panel = self
            .membership
            .keys()
            .find(|id| self.tree.node(id).is_some_and(|node| node.role == "panel"))
            .cloned();
        let panel_range = self.panel.clone().map(|id| self.range(&id));
        self.attachments = self
            .owners
            .values()
            .flat_map(|o| &o.members)
            .filter_map(|id| self.tree.node(id))
            .filter(|n| n.actions.iter().any(|a| a.id == "attachment.save"))
            .count();
        let insert = if self
            .owners
            .get("transcript")
            .is_some_and(|owner| owner.branch)
        {
            self.range("transcript").end
        } else {
            self.order.len()
        };
        let mut streams: Vec<_> = self
            .live
            .keys()
            .filter(|id| {
                !self
                    .tree
                    .contains(id.rsplit_once('.').map(|(owner, _)| owner).unwrap_or(id))
            })
            .cloned()
            .collect();
        streams.sort_by_key(|id| stream_order(id));
        for i in 0..=self.order.len() {
            if i == insert {
                for id in &streams {
                    self.live_positions.insert(id.clone(), self.segments.len());
                    self.segments.push(Segment::Live(id.clone()));
                }
            }
            if let Some(id) = self.order.get(i)
                && !panel_range.as_ref().is_some_and(|range| range.contains(&i))
                && !self.owners[id].footer
                // The session declares the composer so its action can be
                // addressed authoritatively, but the terminal owns its local
                // draft and physical editor rows.
                && self.owners[id].role != "composer"
                && !self.owners[id].lines.is_empty()
            {
                self.segments.push(Segment::Owner(id.clone()));
            }
        }
        self.lengths = self
            .segments
            .iter()
            .map(|segment| self.lines(segment).len())
            .collect();
        for index in (0..self.segments.len()).rev() {
            let rows = self.lines(&self.segments[index]);
            let trimmed = rows
                .iter()
                .rposition(|line| !line.is_blank())
                .map_or(0, |at| at + 1);
            self.lengths[index] = trimmed;
            if trimmed > 0 {
                break;
            }
        }
        self.rows = Rows::new(&self.lengths);
        self.layout_epoch = self.layout_epoch.wrapping_add(1);
    }
    fn lines(&self, segment: &Segment) -> &[Line] {
        match segment {
            Segment::Owner(id) => &self.owners[id].lines,
            Segment::Live(id) => &self.live[id].lines,
        }
    }

    /// The line at a physical row, or `None` for a row outside the document.
    fn row_line(&self, row: usize) -> Option<&Line> {
        let (index, offset, _) = self.rows.locate(row);
        if index >= self.segments.len() {
            return None;
        }
        self.lines(&self.segments[index])[..self.lengths[index]].get(offset)
    }

    /// The anchor for a row: the node it belongs to and how many rows above it in
    /// the same node's run precede it. This is the rewrite's `line-anchor`.
    fn anchor_at(&self, row: usize) -> Anchor {
        let node = self.row_line(row).and_then(|line| line.node.clone());
        let mut offset = 0;
        if node.is_some() {
            let mut previous = row;
            while previous > 0 {
                previous -= 1;
                if self.row_line(previous).and_then(|line| line.node.as_ref()) != node.as_ref() {
                    break;
                }
                offset += 1;
            }
        }
        Anchor { node, offset }
    }

    /// The row an anchor currently occupies, preferring the same run offset. This
    /// is the rewrite's `anchored-row`: it looks for the node without assuming the
    /// physical row survived an update above it.
    fn anchored_row(&self, anchor: &Anchor) -> Option<usize> {
        let want = anchor.node.as_ref()?;
        let mut row = 0;
        for (index, segment) in self.segments.iter().enumerate() {
            for line in &self.lines(segment)[..self.lengths[index]] {
                if line.node.as_deref() == Some(want.as_str()) {
                    return Some(row + anchor.offset);
                }
                row += 1;
            }
        }
        None
    }

    /// Resolve the first row of the viewport.
    ///
    /// Following owns the tail. A reader's scroll owns a physical row until the
    /// layout changes under it; then the anchor decides where they stay. A plain
    /// repaint with no structural change reuses the previous resolution, so
    /// anchoring never walks the document on the animation tick.
    fn viewport_start(&mut self, screen: &Screen, room: usize) -> usize {
        let total = self.rows.total();
        // The last page starts here, so the final row sits on the viewport's last
        // line. Clamping to `total - 1` would leave most of a short page blank and
        // let the composer ride up with the transcript.
        let bottom = total.saturating_sub(room);
        if screen.follow {
            self.following = true;
        }
        let user_scrolled = self.last_intent != screen.scroll_intent;
        if user_scrolled {
            // A reader who scrolls to or past the last page has caught up, so
            // following resumes and later output stays visible.
            self.following = screen.scroll >= bottom;
        }
        if self.following {
            self.anchor = None;
            self.last_intent = screen.scroll_intent;
            self.anchor_row = bottom;
            self.anchor_epoch = self.layout_epoch;
            self.resolved_scroll = bottom;
            return bottom;
        }
        let intent = screen.scroll.min(bottom);
        if user_scrolled || self.anchor.is_none() {
            self.last_intent = screen.scroll_intent;
            self.anchor = Some(self.anchor_at(intent));
            self.anchor_row = intent;
            self.anchor_epoch = self.layout_epoch;
        } else if self.anchor_epoch != self.layout_epoch {
            self.anchor_row = self
                .anchor
                .as_ref()
                .and_then(|anchor| self.anchored_row(anchor))
                .unwrap_or(intent);
            self.anchor_epoch = self.layout_epoch;
        }
        // Reveal the focused selection once per movement. This is the previous
        // system's selection reveal: navigating the document brings the caret into
        // view, and a reader who then scrolls away is not fought on every repaint.
        if let Some(selection) = &screen.selection {
            let head = selection.head();
            if self.revealed_head != Some(head) {
                self.revealed_head = Some(head);
                if head.row < self.anchor_row {
                    self.anchor_row = head.row;
                } else if head.row >= self.anchor_row.saturating_add(room) {
                    self.anchor_row = head.row.saturating_add(1).saturating_sub(room);
                }
                self.anchor = Some(self.anchor_at(self.anchor_row));
                self.anchor_epoch = self.layout_epoch;
            }
        } else {
            self.revealed_head = None;
        }
        self.resolved_scroll = self.anchor_row.min(bottom);
        self.resolved_scroll
    }

    /// The first row the last frame used, for the event loop to write back into
    /// `Screen::scroll` so scroll deltas are relative to the resolved position.
    pub fn resolved_scroll(&self) -> usize {
        self.resolved_scroll
    }

    /// Whether the viewport is showing (and staying on) the tail. The event loop
    /// mirrors it back into `Screen::follow` so every frame agrees.
    pub fn following(&self) -> bool {
        self.following
    }
    fn current(&mut self, stream: Stream, screen: &Screen) {
        let id = stream.id.clone();
        let tail = (thinking_stream(&stream.role) && !screen.prefs.is_open(&id))
            .then_some(THINKING_TAIL_LINES);
        let mut live = Live {
            stream: Stream {
                text: String::new(),
                ..stream.clone()
            },
            lines: vec![],
            document: None,
            rendered: vec![],
            rendered_width: 0,
            last_nonblank: 0,
            tail,
        };
        live.append(&stream.text, screen);
        self.live.insert(id, live);
    }
    /// Apply a complete shared-client document transaction to derived render
    /// caches. Protocol cursors and recovery remain owned by the shared replica.
    pub fn observed(
        &mut self,
        update: &misa_client::document::Update,
        screen: &Screen,
    ) -> Result<(), String> {
        use misa_client::document::Update;
        use misa_protocol::observation::{Applied, MemberChange};
        match update {
            Update::Reset(document) => {
                *self = Self::new(document.tree.clone(), screen);
                for stream in &document.streams {
                    self.current(stream.clone(), screen);
                }
                self.reindex();
            }
            Update::Changed { member, applied } => {
                let Applied::Changed(members) = applied.as_ref() else {
                    return Err("Expected document transaction".into());
                };
                let Some(MemberChange::Document {
                    tree,
                    live,
                    reset_live,
                }) = members.get(member)
                else {
                    return Err("Expected document member changes".into());
                };
                let mut structural = !tree.is_empty() || *reset_live;
                for op in tree {
                    self.op(op, screen)?;
                }
                if *reset_live {
                    self.live.clear();
                }
                for update in live {
                    match update {
                        StreamUpdate::Current { stream } => {
                            self.current(stream.clone(), screen);
                            structural = true;
                        }
                        StreamUpdate::End { id } => {
                            self.live.remove(id);
                            structural = true;
                        }
                        StreamUpdate::Append { id, offset, text } => {
                            let live = self.live.get_mut(id).ok_or("Missing rendered stream")?;
                            if live.stream.text.len() != *offset {
                                return Err("Rendered stream offset gap".into());
                            }
                            live.append(text, screen);
                            self.work.appended_bytes += text.len();
                            if !structural {
                                if let Some(&index) = self.live_positions.get(id) {
                                    let new = if self.rows.total() == self.rows.prefix(index + 1) {
                                        live.last_nonblank
                                    } else {
                                        live.lines.len()
                                    };
                                    self.work.index_steps +=
                                        self.rows.change(index, self.lengths[index], new);
                                    self.lengths[index] = new;
                                    self.layout_epoch = self.layout_epoch.wrapping_add(1);
                                }
                            }
                        }
                    }
                }
                if structural {
                    self.reindex();
                }
            }
            Update::Unavailable(_) | Update::Status(_) => {}
        }
        Ok(())
    }
    pub fn local(&mut self, screen: &Screen) {
        if self.width != screen.width
            || self.theme != screen.theme.name
            || self.opened != screen.prefs.opened
            || self.components != screen.prefs.components
        {
            let streams: Vec<_> = self.live.values().map(|live| live.stream.clone()).collect();
            *self = Self::new(self.tree.snapshot(), screen);
            for stream in streams {
                self.current(stream, screen);
            }
            self.reindex();
        } else if let Some(panel) = self.panel.clone() {
            if let Some(id) = self.membership.get(&panel).cloned() {
                self.refresh(&id, screen);
                self.reindex();
            }
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
        let document = self
            .segments
            .iter()
            .enumerate()
            .flat_map(|(index, segment)| self.lines(segment)[..self.lengths[index]].iter().cloned())
            .take(limit)
            .collect();
        let footer = self
            .order
            .iter()
            .filter_map(|id| self.owners.get(id))
            .filter(|owner| owner.footer)
            .flat_map(|owner| owner.lines.iter().cloned())
            .take(limit)
            .collect();
        (document, footer)
    }
    fn panel_lines(&self) -> Vec<Line> {
        let Some(panel) = &self.panel else {
            return Vec::new();
        };
        self.range(panel)
            .skip(1)
            .filter_map(|index| self.order.get(index).and_then(|id| self.owners.get(id)))
            .filter(|owner| !owner.footer)
            .flat_map(|owner| owner.lines.iter().cloned())
            .collect()
    }
    fn surface_lines(&self, screen: &Screen) -> Vec<Line> {
        let Some(panel) = &self.panel else {
            return Vec::new();
        };
        let title = self
            .tree
            .node(panel)
            .and_then(|node| node.label.clone())
            .unwrap_or_else(|| "Interaction".into());
        let mut lines = vec![Line {
            surface: screen.theme.surface("dialog"),
            indent: 0,
            node: None,
            spans: vec![
                (screen.theme.role("dialog.label"), "┌─ ".into()),
                (screen.theme.role("dialog.title"), title),
            ],
        }];
        for mut line in self.panel_lines() {
            line.surface = line.surface.or_else(|| screen.theme.surface("dialog"));
            line.spans
                .insert(0, (screen.theme.role("dialog.label"), "│ ".into()));
            lines.push(line);
        }
        let actions = self
            .tree
            .node(panel)
            .into_iter()
            .flat_map(|node| {
                node.actions
                    .iter()
                    .chain(node.children.iter().flat_map(|child| child.actions.iter()))
            })
            .filter(|action| matches!(action.on, ActionOn::Click | ActionOn::Submit))
            .map(|action| {
                (
                    action.id.as_str(),
                    action.label.as_deref().unwrap_or(action.id.as_str()),
                )
            })
            .collect::<Vec<_>>();
        let mut footer = crate::buttons::footer(&screen.theme, &screen.prefs.dialogs, actions);
        footer.surface = screen.theme.surface("dialog");
        lines.push(footer);
        lines
    }
    pub fn frame_with(
        &mut self,
        screen: &Screen,
        staging: Option<&str>,
        extra_document: &[Line],
        extra_footer: &[Line],
    ) -> crate::chrome::Frame {
        let mut top = crate::chrome::top(screen, self.attachments, None);
        if screen.dialogs.modal() || self.panel.is_some() {
            // Dialogs and session panels are overlays. They own input focus, but
            // do not erase the transcript or the status bar beneath them.
            let status: Vec<_> = self
                .order
                .iter()
                .filter_map(|id| self.owners.get(id))
                .filter(|owner| owner.footer && owner.role != "queue")
                .flat_map(|owner| owner.lines.iter().cloned())
                .chain(extra_footer.iter().cloned())
                .take(1)
                .collect();
            top.truncate(screen.height as usize);
            let available = (screen.height as usize).saturating_sub(top.len() + status.len());
            let overlay = if screen.dialogs.modal() {
                screen
                    .dialogs
                    .lines(&screen.theme, screen.width as usize, &screen.prefs.dialogs)
            } else {
                self.surface_lines(screen)
            };
            let overlay = crate::chrome::physical(overlay, screen.width as usize);
            let overlay_len = overlay.len().min(available);
            let room = available.saturating_sub(overlay_len);
            let start = self.viewport_start(screen, room);
            let (mut segment, mut offset, steps) = self.rows.locate(start);
            self.work.index_steps += steps;
            let mut middle = Vec::new();
            while middle.len() < room && segment < self.segments.len() {
                let rows = &self.lines(&self.segments[segment])[..self.lengths[segment]];
                let take = (room - middle.len()).min(rows.len().saturating_sub(offset));
                middle.extend_from_slice(&rows[offset..offset + take]);
                segment += 1;
                offset = 0;
            }
            middle.extend_from_slice(
                &extra_document[..extra_document.len().min(room.saturating_sub(middle.len()))],
            );
            let cursor_row = top.len() + middle.len();
            let cursor_column = overlay
                .first()
                .map(|line| {
                    misa_render::width(&line.text()).min(screen.width.saturating_sub(1) as usize)
                })
                .unwrap_or(0);
            middle.extend(overlay.into_iter().take(overlay_len));
            let mut lines = top;
            lines.extend(middle);
            lines.extend(status);
            return crate::chrome::Frame {
                lines,
                cursor_row,
                cursor_column,
            };
        }
        if screen
            .picker
            .as_ref()
            .is_some_and(misa_kit::picker::Picker::is_overlay)
        {
            // The reference picker is an overlay: the transcript remains visible
            // above it, the picker occupies the available middle rows, and status
            // remains the bottom region. The normal composer is intentionally not
            // part of this layout while the picker owns input focus.
            let status: Vec<_> = self
                .order
                .iter()
                .filter_map(|id| self.owners.get(id))
                .filter(|owner| owner.footer && owner.role != "queue")
                .flat_map(|owner| owner.lines.iter().cloned())
                .collect();
            let status = status
                .into_iter()
                .chain(extra_footer.iter().cloned())
                .take(1)
                .collect::<Vec<_>>();
            let picker = crate::chrome::physical(
                crate::picker_lines(screen, screen.picker.as_ref().unwrap()),
                screen.width as usize,
            );
            let available = (screen.height as usize).saturating_sub(top.len() + status.len());
            let picker_len = picker.len().min(available);
            let room = available.saturating_sub(picker_len);
            let start = self.viewport_start(screen, room);
            let (mut segment, mut offset, steps) = self.rows.locate(start);
            self.work.index_steps += steps;
            let mut lines = Vec::new();
            while lines.len() < room && segment < self.segments.len() {
                let rows = &self.lines(&self.segments[segment])[..self.lengths[segment]];
                let take = (room - lines.len()).min(rows.len().saturating_sub(offset));
                lines.extend_from_slice(&rows[offset..offset + take]);
                segment += 1;
                offset = 0;
            }
            // Contributions are document furniture, so an overlay must not make
            // them disappear. They consume the same document room as the retained
            // transcript, just as they do in the normal frame.
            lines.extend_from_slice(
                &extra_document[..extra_document.len().min(room.saturating_sub(lines.len()))],
            );
            let cursor_column = screen
                .picker
                .as_ref()
                .map(|picker| {
                    let prefix = match picker.accept {
                        misa_kit::picker::Accept::Run => "/",
                        misa_kit::picker::Accept::Action { .. } => ":",
                        misa_kit::picker::Accept::Argument { .. } => "",
                    };
                    let query = format!("  {}: {}{}", picker.title, prefix, picker.query);
                    misa_render::width(&query).min(screen.width.saturating_sub(1) as usize)
                })
                .unwrap_or(0);
            let cursor_row = top.len() + lines.len();
            lines.extend(picker.into_iter().take(picker_len));
            lines.extend(status);
            let mut frame_lines = top;
            frame_lines.extend(lines);
            return crate::chrome::Frame {
                lines: frame_lines,
                cursor_row,
                cursor_column,
            };
        }
        let preferred_input = crate::chrome::composer(screen);
        let dock: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| self.owners.get(id))
            .filter(|owner| owner.footer && owner.role == "queue")
            .flat_map(|owner| owner.lines.iter().cloned())
            .collect();
        let dock = dock
            .into_iter()
            .chain(staging.into_iter().map(|text| Line {
                surface: None,
                indent: 0,
                node: None,
                spans: vec![(screen.theme.role("keybinding"), text.to_string())],
            }))
            .collect::<Vec<_>>();
        let dock = crate::chrome::physical(dock, screen.width as usize);
        let status: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| self.owners.get(id))
            .filter(|owner| owner.footer && owner.role != "queue")
            .flat_map(|owner| owner.lines.iter().cloned())
            .collect();
        // The reference reserves one status row, the queue immediately above the
        // editor, and at least one row for the transcript when the terminal allows
        // it. The editor itself is capped at half the terminal height.
        let status = status
            .into_iter()
            .chain(extra_footer.iter().cloned())
            .take(1)
            .collect::<Vec<_>>();
        let available = (screen.height as usize).saturating_sub(top.len() + status.len());
        let editor_rows = preferred_input
            .lines
            .len()
            .min((screen.height as usize / 2).max(1))
            .min(available);
        let input = crate::chrome::composer_with_budget(screen, editor_rows);
        // One blank row separates the transcript (and its dock) from the composer.
        // It is reserved before the transcript so the composer's rows are fixed.
        let separator = usize::from(available > editor_rows);
        let remaining = available
            .saturating_sub(editor_rows)
            .saturating_sub(separator);
        let dock = &dock[..dock.len().min(remaining.saturating_sub(1))];
        let remaining = remaining.saturating_sub(dock.len());
        let completions = screen
            .picker
            .as_ref()
            .filter(|picker| picker.is_inline())
            .map(|picker| crate::completion_lines(screen, picker))
            .unwrap_or_default();
        // The reference gives inline completions the second half of the space
        // left after the dock, leaving the first half for transcript rows.
        let completion_count = completions.len().min(remaining / 2);
        let room = remaining.saturating_sub(completion_count);
        let extra_document = &extra_document[..extra_document.len().min(room)];
        let room = room.saturating_sub(extra_document.len());
        let start = self.viewport_start(screen, room);
        let (mut segment, mut offset, steps) = self.rows.locate(start);
        self.work.index_steps += steps;
        let mut lines = vec![];
        while lines.len() < room && segment < self.segments.len() {
            let rows = &self.lines(&self.segments[segment])[..self.lengths[segment]];
            let take = (room - lines.len()).min(rows.len().saturating_sub(offset));
            lines.extend_from_slice(&rows[offset..offset + take]);
            segment += 1;
            offset = 0;
        }
        self.work.copied_rows += lines.len();
        if let Some(selection) = &screen.selection {
            // Selection is explicit local work. It uses cached rows, never tree formatting.
            if self.selection_body.is_none() {
                let all: Vec<_> = self
                    .segments
                    .iter()
                    .enumerate()
                    .flat_map(|(i, segment)| self.lines(segment)[..self.lengths[i]].iter().cloned())
                    .collect();
                self.selection_body = Some(crate::select::Body::of(&all));
            }
            for (i, line) in lines.iter_mut().enumerate() {
                if let Some((from, to)) =
                    selection.on_row(self.selection_body.as_ref().unwrap(), start + i)
                {
                    crate::select_highlight(line, from, to, &screen.theme);
                }
            }
        }
        lines.extend_from_slice(extra_document);
        // Session-owned input docks (for example the queued-prompt preview) belong
        // immediately above the composer. Status is a bottom bar in the reference
        // layout, so independently composed status documents stay after the chrome.
        let cursor_row = top.len()
            + lines.len()
            + dock.len()
            + separator
            + input.cursor_row.min(editor_rows.saturating_sub(1));
        lines.extend_from_slice(dock);
        if separator > 0 {
            lines.push(Line::default());
        }
        lines.extend(input.lines.into_iter().take(editor_rows));
        lines.extend(completions.into_iter().take(completion_count));
        let mut frame_lines = top;
        frame_lines.extend(lines);
        frame_lines.extend(status);
        crate::chrome::Frame {
            lines: frame_lines,
            cursor_row,
            cursor_column: input.cursor_column,
        }
    }
}

/// Resolved presentation of one live stream at the current terminal width.
struct Paint {
    rail: Option<(String, Style)>,
    surface: Option<Style>,
    width: usize,
}

impl Paint {
    fn of(screen: &Screen, role: &str) -> Self {
        let rail = screen.theme.rail(role);
        let inset = rail
            .as_ref()
            .map_or(0, |(glyph, _)| misa_render::width(glyph));
        Self {
            surface: screen.theme.surface(role),
            rail,
            width: (screen.width as usize).saturating_sub(inset).max(1),
        }
    }
}

impl Live {
    fn append(&mut self, text: &str, screen: &Screen) {
        self.stream.text.push_str(text);
        let paint = Paint::of(screen, &self.stream.role);
        // Every live stream, collapsed thinking included, renders through the
        // markdown path a settled body uses. The tail is a view over the result.
        self.markdown(&paint, &screen.theme);
    }

    /// Parse the in-flight text as markdown and lay it out, reusing the previous
    /// parse's blocks where it can.
    fn markdown(&mut self, paint: &Paint, theme: &Theme) {
        let text = std::mem::take(&mut self.stream.text);
        let id = self.stream.id.clone();
        let document = misa_markdown::document(&self.stream.role, &text, self.document.as_ref());
        let changed = self
            .document
            .as_ref()
            .is_none_or(|previous| previous.source != document.source);
        if changed {
            let width = paint.width;
            let reuse = self.rendered_width == width;
            let mut rendered = Vec::with_capacity(document.blocks.len());
            for (index, block) in document.blocks.iter().enumerate() {
                if let Some((node, lines)) = self.rendered.get(index)
                    && reuse
                    && node == block
                {
                    rendered.push((node.clone(), lines.clone()));
                    continue;
                }
                let lines = misa_lines::render_block(block, theme, width, 0)
                    .into_iter()
                    .map(|mut line| {
                        line.node = Some(id.clone());
                        if let Some((glyph, rail_style)) = &paint.rail {
                            line.spans.insert(0, (*rail_style, glyph.clone()));
                        }
                        line.surface = line.surface.or(paint.surface);
                        line
                    })
                    .collect();
                rendered.push((block.clone(), lines));
            }
            self.lines = rendered
                .iter()
                .flat_map(|(_, lines)| lines.iter().cloned())
                .collect();
            self.rendered = rendered;
            self.rendered_width = width;
            // A collapsed thinking stream is a window onto the tail of the same
            // lines every other stream renders. Keeping only the last rows here
            // means the window holds no rows the viewport can scroll to.
            if let Some(limit) = self.tail {
                let skip = self.lines.len().saturating_sub(limit);
                self.lines.drain(..skip);
            }
            self.last_nonblank = self
                .lines
                .iter()
                .rposition(|line| !line.is_blank())
                .map_or(0, |index| index + 1);
        }
        self.document = Some(document);
        self.stream.text = text;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn independent_documents_keep_their_own_node_identities_and_placement() {
        let screen = crate::Screen::new(80, 20);
        let make = |text: &str| {
            misa_proto::Node::text("notice", [misa_proto::view::Span::plain(text)]).id("same")
        };
        let mut left = super::Retained::new(make("left"), &screen);
        let right = super::Retained::new(make("right"), &screen);
        left.op(
            &misa_proto::sync::ViewOp::Replace {
                id: "same".into(),
                node: make("changed"),
            },
            &screen,
        )
        .unwrap();
        left.reindex();
        assert!(
            left.placed_lines(20)
                .0
                .iter()
                .any(|line| line.text().contains("changed"))
        );
        assert!(
            right
                .placed_lines(20)
                .0
                .iter()
                .any(|line| line.text().contains("right"))
        );
        let mut main = super::Retained::new(make("conversation"), &screen);
        let extras = left
            .placed_lines(20)
            .0
            .into_iter()
            .chain(right.placed_lines(20).0)
            .collect::<Vec<_>>();
        let frame = main.frame_with(&screen, None, &extras, &[]);
        assert!(
            frame
                .lines
                .iter()
                .any(|line| line.text().contains("conversation"))
        );
        assert!(
            frame
                .lines
                .iter()
                .any(|line| line.text().contains("changed"))
        );
        assert!(frame.lines.iter().any(|line| line.text().contains("right")));
    }
    use super::*;
    use misa_proto::view::Span;
    fn text(id: &str, value: &str) -> Node {
        Node::text("assistant", [Span::plain(value)]).id(id)
    }
    fn document(history: usize) -> Node {
        let mut transcript = Node::section("transcript").id("transcript");
        for i in 0..history {
            transcript.children.push(
                Node::section("message")
                    .id(format!("msg{i}"))
                    .child(text(&format!("msg{i}.body"), "settled content")),
            );
        }
        Node::section("session")
            .id("session")
            .child(text("header", "Header"))
            .child(transcript)
    }
    fn all(retained: &Retained) -> Vec<Line> {
        let mut lines: Vec<_> = retained
            .segments
            .iter()
            .flat_map(|segment| retained.lines(segment).iter().cloned())
            .collect();
        while lines.last().is_some_and(Line::is_blank) {
            lines.pop();
        }
        lines
    }
    fn oracle(retained: &Retained, screen: &Screen) {
        assert_eq!(
            all(retained),
            misa_lines::render(
                &screen.resolve(&retained.tree.snapshot()),
                &screen.theme,
                screen.width as usize
            )
        );
    }
    #[test]
    fn a_scrolled_viewport_keeps_its_node_when_content_arrives_above() {
        use misa_proto::sync::ViewOp;
        let mut screen = Screen::new(40, 12);
        let document = |count: usize| {
            let mut root = Node::section("session").id("session");
            for index in 0..count {
                root.children
                    .push(text(&format!("t{index}"), &format!("line {index}")));
            }
            root
        };
        let mut retained = Retained::new(document(12), &screen);
        screen.follow = false;
        screen.scroll = 3;
        screen.scroll_intent = 1;
        let start = retained.viewport_start(&screen, 5);
        assert_eq!(start, 3);
        assert_eq!(
            retained
                .row_line(start)
                .and_then(|line| line.node.as_deref()),
            Some("t3")
        );
        // A line arrives above the reader. The physical row shifts; the anchor does
        // not, so the frame starts at the same node it did before.
        retained
            .op(
                &ViewOp::Insert {
                    parent: "session".into(),
                    before: Some("t0".into()),
                    node: text("new", "line new"),
                },
                &screen,
            )
            .unwrap();
        retained.reindex();
        let after = retained.viewport_start(&screen, 5);
        assert_eq!(after, 4, "the anchor moved down with its node");
        assert_eq!(
            retained
                .row_line(after)
                .and_then(|line| line.node.as_deref()),
            Some("t3")
        );
    }

    #[test]
    fn a_plain_repaint_does_not_rewalk_the_document_and_a_user_scroll_reanchors() {
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(6), &screen);
        screen.follow = false;
        screen.scroll = 2;
        screen.scroll_intent = 1;
        let first = retained.viewport_start(&screen, 3);
        assert_eq!(first, 2);
        // No new intent and no layout change: the resolution is reused.
        retained.work = Work::default();
        let again = retained.viewport_start(&screen, 3);
        assert_eq!(again, 2);
        assert_eq!(retained.work, Work::default());
        // A reader delta re-anchors against the row they asked for.
        screen.scroll = 4;
        screen.scroll_intent = 2;
        let scrolled = retained.viewport_start(&screen, 3);
        assert_eq!(scrolled, 4);
        assert_eq!(
            retained
                .row_line(scrolled)
                .and_then(|line| line.node.as_deref()),
            Some("msg3.body")
        );
    }

    #[test]
    fn moving_the_selection_reveals_it_once_without_fighting_a_later_scroll() {
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(20), &screen);
        screen.follow = false;
        screen.scroll = 0;
        screen.scroll_intent = 1;
        assert_eq!(retained.viewport_start(&screen, 4), 0);
        // The caret moves below the viewport; the frame scrolls it into view.
        screen.selection = Some(crate::select::Selection::caret(crate::select::Spot::new(
            10, 0,
        )));
        assert_eq!(retained.viewport_start(&screen, 4), 7);
        // The same selection on the next repaint does not move the viewport again.
        assert_eq!(retained.viewport_start(&screen, 4), 7);
        // A reader's scroll is respected; the unchanged caret does not pull back.
        screen.scroll = 2;
        screen.scroll_intent = 2;
        assert_eq!(retained.viewport_start(&screen, 4), 2);
        // Moving the caret again reveals the new position.
        screen.selection = Some(crate::select::Selection::caret(crate::select::Spot::new(
            2, 0,
        )));
        assert_eq!(retained.viewport_start(&screen, 4), 2);
    }

    #[test]
    fn following_tracks_the_tail_and_ignores_the_anchor() {
        let screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(6), &screen);
        let total = retained.rows.total();
        assert!(screen.follow);
        assert_eq!(retained.viewport_start(&screen, 3), total.saturating_sub(3));
        assert!(retained.anchor.is_none());
    }

    #[test]
    fn a_scroll_clamps_to_the_last_page_and_resumes_following_at_the_bottom() {
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(20), &screen);
        let room = 4;
        let total = retained.rows.total();
        let bottom = total.saturating_sub(room);
        screen.follow = false;
        // A scroll well past the end lands on the last full page, never on
        // `total - 1` with a nearly empty viewport.
        screen.scroll = total + 5;
        screen.scroll_intent = 1;
        assert_eq!(retained.viewport_start(&screen, room), bottom);
        assert!(
            retained.following(),
            "reaching the bottom must resume following"
        );
        // New output then tracks the tail, because following is back on.
        retained
            .op(
                &ViewOp::Insert {
                    parent: "transcript".into(),
                    before: None,
                    node: text("extra", "extra"),
                },
                &screen,
            )
            .unwrap();
        retained.reindex();
        let grown = retained.rows.total();
        assert_eq!(
            retained.viewport_start(&screen, room),
            grown.saturating_sub(room)
        );
    }

    #[test]
    fn the_wheel_scroll_handler_clamps_at_the_ends_and_resumes_following() {
        // The mouse wheel maps to `Key::ScrollPage`, the same handler the keyboard
        // scroll path uses. Driving it here is driving the wheel.
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(20), &screen);
        let room = 4;
        let total = retained.rows.total();
        let bottom = total.saturating_sub(room);
        screen.follow = false;
        // A wheel-up at the top cannot scroll past row zero.
        screen.key(crate::Key::ScrollPage(-30));
        assert_eq!(retained.viewport_start(&screen, room), 0);
        assert!(!retained.following());
        // Walking down to the last page resumes following.
        loop {
            screen.key(crate::Key::ScrollPage(3));
            let start = retained.viewport_start(&screen, room);
            screen.scroll = start;
            if start == bottom {
                break;
            }
        }
        assert!(retained.following());
        // One more notch stays clamped to the last page.
        screen.key(crate::Key::ScrollPage(3));
        assert_eq!(retained.viewport_start(&screen, room), bottom);
    }

    #[test]
    fn the_composer_keeps_its_row_when_the_transcript_scrolls() {
        let mut screen = Screen::new(40, 16);
        screen.editor.set_text("DRAFT");
        let mut retained = Retained::new(document(50), &screen);
        let composer_row = |frame: &crate::chrome::Frame| {
            frame
                .lines
                .iter()
                .position(|line| line.text().contains("DRAFT"))
                .expect("the composer")
        };
        let following = composer_row(&retained.frame(&screen, None));
        // Scrolling up and then back to the tail must not move the composer: it
        // is reserved at the bottom before the transcript gets its room.
        screen.follow = false;
        screen.scroll = 4;
        screen.scroll_intent = 1;
        assert_eq!(composer_row(&retained.frame(&screen, None)), following);
        screen.scroll = retained.rows.total();
        screen.scroll_intent = 2;
        assert_eq!(composer_row(&retained.frame(&screen, None)), following);
    }

    #[test]
    fn shared_document_updates_keep_token_work_incremental_and_settle_atomically() {
        use misa_client::document::Update;
        use misa_protocol::observation::{Applied, MemberChange};
        let update = |tree, live| Update::Changed {
            member: "body".into(),
            applied: std::sync::Arc::new(Applied::Changed(std::collections::BTreeMap::from([(
                "body".into(),
                MemberChange::Document {
                    tree,
                    live,
                    reset_live: false,
                },
            )]))),
        };
        for history in [100, 1000] {
            let screen = Screen::new(40, 16);
            let mut retained = Retained::new(document(history), &screen);
            retained.current(
                Stream {
                    id: "new.body".into(),
                    role: "assistant".into(),
                    text: String::new(),
                },
                &screen,
            );
            retained.reindex();
            retained.work = Work::default();
            retained
                .observed(
                    &update(
                        vec![],
                        vec![StreamUpdate::Append {
                            id: "new.body".into(),
                            offset: 0,
                            text: "é🙂".into(),
                        }],
                    ),
                    &screen,
                )
                .unwrap();
            assert_eq!(retained.work.formatted_nodes, 0);
            assert_eq!(retained.work.appended_bytes, 6);
            assert!(retained.work.index_steps <= 24);
            retained
                .observed(
                    &update(
                        vec![ViewOp::Insert {
                            parent: "transcript".into(),
                            before: None,
                            node: text("new", "é🙂"),
                        }],
                        vec![StreamUpdate::End {
                            id: "new.body".into(),
                        }],
                    ),
                    &screen,
                )
                .unwrap();
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
                ViewOp::Insert {
                    parent: "transcript".into(),
                    before: Some("msg4".into()),
                    node: text("inserted", "inserted at a stable anchor"),
                },
                ViewOp::Replace {
                    id: "msg3.body".into(),
                    node: text(
                        "msg3.body",
                        "a much longer replacement that wraps into multiple lines at this width",
                    ),
                },
                ViewOp::Remove { id: "msg7".into() },
                ViewOp::Insert {
                    parent: "msg4".into(),
                    before: None,
                    node: text("msg4.extra", "another body"),
                },
                ViewOp::Replace {
                    id: "msg4".into(),
                    node: text("msg4", "section becomes content"),
                },
            ];
            for op in operations {
                retained.op(&op, &screen).unwrap();
                retained.reindex();
                oracle(&retained, &screen);
            }
            screen.width = 19;
            retained.local(&screen);
            oracle(&retained, &screen);
        }
    }

    #[test]
    fn message_sections_are_rendered_as_one_rail_aware_owner() {
        let screen = Screen::new(40, 12);
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::section("message.assistant").id("message.1").child(
                    Node::text(
                        "message.assistant.markdown.paragraph",
                        [Span::plain("hello")],
                    )
                    .id("message.1.body"),
                ),
            ),
        );
        let retained = Retained::new(view.clone(), &screen);
        let lines = all(&retained);
        assert!(
            lines.iter().any(|line| line.text().contains("┃ hello")),
            "{lines:?}"
        );
        assert_eq!(
            lines,
            misa_lines::render(&screen.resolve(&view), &screen.theme, screen.width as usize)
        );
    }

    #[test]
    fn a_live_stream_is_drawn_as_the_block_it_will_settle_into() {
        let screen = Screen::new(40, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "pondering".into(),
            },
            &screen,
        );
        retained.reindex();
        let lines = all(&retained);
        let body = lines
            .iter()
            .find(|line| line.text().contains("pondering"))
            .expect("the streamed body");
        assert!(body.text().starts_with("┃ "), "{:?}", body.text());
        assert_eq!(
            body.surface,
            screen.theme.surface("message.assistant.thinking")
        );
    }

    #[test]
    fn a_pending_messages_thinking_stream_renders_before_its_text() {
        let screen = Screen::new(40, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.text".into(),
                role: "message.assistant".into(),
                text: "answer".into(),
            },
            &screen,
        );
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "reason".into(),
            },
            &screen,
        );
        retained.reindex();
        let rows: Vec<String> = all(&retained).iter().map(Line::text).collect();
        let thinking = rows.iter().position(|row| row.contains("reason")).unwrap();
        let answer = rows.iter().position(|row| row.contains("answer")).unwrap();
        assert!(thinking < answer, "{rows:?}");
    }

    #[test]
    fn a_streamed_answer_renders_markdown_not_plain_text() {
        let screen = Screen::new(60, 20);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.text".into(),
                role: "message.assistant".into(),
                text: "a **bold** word\n\n```rust\nlet x = 1;\n```".into(),
            },
            &screen,
        );
        retained.reindex();
        let lines = all(&retained);
        let rows: Vec<String> = lines.iter().map(Line::text).collect();
        // The bold run is styled, not shown as `**bold**`.
        assert!(
            lines.iter().any(|line| line
                .spans
                .iter()
                .any(|(style, text)| style.bold && text.trim() == "bold")),
            "{:?}",
            lines
                .iter()
                .flat_map(|line| line
                    .spans
                    .iter()
                    .map(|(style, text)| (format!("{style:?}"), text.clone())))
                .collect::<Vec<_>>()
        );
        assert!(!rows.iter().any(|row| row.contains("**")), "{rows:?}");
        // The fence is a code block, with its language label and body.
        assert!(rows.iter().any(|row| row.contains("rust")), "{rows:?}");
        assert!(rows.iter().any(|row| row.contains("let x")), "{rows:?}");
    }

    #[test]
    fn an_incrementally_streamed_answer_rewraps_only_its_last_segment() {
        let screen = Screen::new(20, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.body".into(),
                role: "message.assistant".into(),
                text: String::new(),
            },
            &screen,
        );
        let live = retained.live.get_mut("msg1.body").unwrap();
        for delta in ["alpha ", "beta\n", "gamma ", "delta"] {
            live.append(delta, &screen);
        }
        assert_eq!(
            live.lines.iter().map(Line::text).collect::<Vec<_>>(),
            vec!["┃ alpha beta", "┃ gamma delta"]
        );
    }

    #[test]
    fn a_collapsed_thinking_tail_renders_markdown_like_the_expanded_stream() {
        let text = "intro line one\nintro line two\n\na **bold** word\n\n```rust\nlet x = 1;\n```";
        let stream = || Stream {
            id: "msg1.thinking".into(),
            role: "message.assistant.thinking".into(),
            text: text.into(),
        };
        let screen = Screen::new(40, 20);
        let mut collapsed = Retained::new(Node::section("session").id("session"), &screen);
        collapsed.current(stream(), &screen);
        collapsed.reindex();
        let tail = all(&collapsed);

        let mut opened = Screen::new(40, 20);
        opened.prefs.opened = vec!["msg1.thinking".into()];
        let mut expanded = Retained::new(Node::section("session").id("session"), &opened);
        expanded.current(stream(), &opened);
        expanded.reindex();
        let full = all(&expanded);
        // The collapsed window is exactly the last rows of the same rendering.
        assert_eq!(tail, full[full.len() - THINKING_TAIL_LINES..].to_vec());
        assert_eq!(tail.len(), THINKING_TAIL_LINES);
        // The window still carries markdown: the bold run is styled.
        assert!(
            tail.iter().any(|line| line
                .spans
                .iter()
                .any(|(style, text)| style.bold && text.trim() == "bold")),
            "{tail:?}"
        );
        // And the fenced block is laid out, language label and code row included.
        let rows: Vec<String> = tail.iter().map(Line::text).collect();
        assert!(rows.iter().any(|row| row.contains("rust")), "{rows:?}");
        assert!(
            rows.iter().any(|row| row.contains("let x = 1;")),
            "{rows:?}"
        );
    }

    #[test]
    fn resizing_a_collapsed_markdown_tail_reflows_and_keeps_the_last_rows() {
        let text = "```rust\nlet x = 1;\n```\n\nalpha **bold** beta gamma delta epsilon zeta eta theta iota kappa";
        let stream = || Stream {
            id: "msg1.thinking".into(),
            role: "message.assistant.thinking".into(),
            text: text.into(),
        };
        let mut screen = Screen::new(40, 20);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(stream(), &screen);
        retained.reindex();
        let wide = all(&retained);

        // A narrower terminal rewraps the unified rendering and re-applies the
        // window, so the visible rows are still the most recent reasoning.
        screen.width = 24;
        retained.local(&screen);
        let narrow = all(&retained);
        assert_ne!(wide, narrow, "the stream must reflow at the new width");
        assert_eq!(narrow.len(), THINKING_TAIL_LINES);

        let mut opened = Screen::new(24, 20);
        opened.prefs.opened = vec!["msg1.thinking".into()];
        let mut expanded = Retained::new(Node::section("session").id("session"), &opened);
        expanded.current(stream(), &opened);
        expanded.reindex();
        let full = all(&expanded);
        assert_eq!(narrow, full[full.len() - THINKING_TAIL_LINES..].to_vec());
    }

    #[test]
    fn a_collapsed_thinking_stream_follows_its_tail() {
        let screen = Screen::new(40, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "one\ntwo\nthree\nfour\nfive".into(),
            },
            &screen,
        );
        retained.reindex();
        let rows: Vec<String> = all(&retained).iter().map(Line::text).collect();
        assert_eq!(rows, vec!["┃ three", "┃ four", "┃ five"]);
    }

    #[test]
    fn resizing_a_tailed_thinking_stream_reflows_and_keeps_the_tail() {
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "alpha alpha\nalpha beta\nalpha gamma".into(),
            },
            &screen,
        );
        retained.reindex();
        assert_eq!(
            all(&retained).iter().map(Line::text).collect::<Vec<_>>(),
            vec!["┃ alpha alpha", "┃ alpha beta", "┃ alpha gamma"]
        );
        // A narrower terminal rewraps the buffered text, word-aware, and re-applies
        // the tail, so the visible window is still the most recent reasoning.
        screen.width = 10;
        retained.local(&screen);
        assert_eq!(
            all(&retained).iter().map(Line::text).collect::<Vec<_>>(),
            vec!["┃ beta", "┃ alpha", "┃ gamma"]
        );
    }

    #[test]
    fn a_collapsed_thinking_tail_wraps_at_word_boundaries() {
        let screen = Screen::new(10, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "alpha gamma".into(),
            },
            &screen,
        );
        retained.reindex();
        assert_eq!(
            all(&retained).iter().map(Line::text).collect::<Vec<_>>(),
            vec!["┃ alpha", "┃ gamma"]
        );
    }

    #[test]
    fn an_opened_thinking_stream_keeps_every_line() {
        let mut screen = Screen::new(40, 12);
        screen.prefs.opened = vec!["msg1.thinking".into()];
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "one\ntwo\nthree\nfour\nfive".into(),
            },
            &screen,
        );
        retained.reindex();
        let rows: Vec<String> = all(&retained).iter().map(Line::text).collect();
        assert_eq!(rows, vec!["┃ one", "┃ two", "┃ three", "┃ four", "┃ five"]);
    }

    #[test]
    fn an_opened_thinking_block_keeps_one_rail_not_the_parents() {
        let mut screen = Screen::new(60, 16);
        screen.prefs.opened = vec!["msg.1.thinking".into()];
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::section("message.assistant").id("msg.1").child(
                    Node::new(
                        "message.assistant.thinking",
                        Kind::Collapsible {
                            summary: vec![Span::strong("thinking")],
                        },
                    )
                    .id("msg.1.thinking")
                    .child(Node::text(
                        "message.assistant.thinking.text",
                        [Span::plain("one\ntwo")],
                    )),
                ),
            ),
        );
        let retained = Retained::new(view, &screen);
        let lines = all(&retained);
        let rows: Vec<String> = lines.iter().map(Line::text).collect();
        // The assistant's padding, then the thinking block's own padding and body.
        assert_eq!(rows, vec!["┃ ", "┃ ", "┃ one", "┃ two", "┃ ", "", "┃ "]);
        // The rail is the thinking block's own, not the assistant's stacked on it.
        assert_eq!(
            lines[1].spans[0].0,
            screen.theme.role("message.assistant.thinking.rail")
        );
    }

    #[test]
    fn a_closed_tool_call_shows_one_name_under_its_own_rail() {
        let screen = Screen::new(60, 16);
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::section("message.assistant").id("msg.1").child(
                    Node::new(
                        "tool.call",
                        Kind::Collapsible {
                            summary: vec![Span::strong("echo"), Span::plain(" ")],
                        },
                    )
                    .id("msg.1.call.1")
                    .label("echo")
                    .child(Node::text("tool.call.args", [Span::plain("hi")])),
                ),
            ),
        );
        let retained = Retained::new(view, &screen);
        let rows: Vec<String> = all(&retained).iter().map(Line::text).collect();
        assert_eq!(rows, vec!["┃ ", "┃ ◇ echo", "┃ "]);
    }

    #[test]
    fn a_closed_thinking_block_previews_a_few_lines_under_the_rail() {
        let screen = Screen::new(60, 16);
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::section("message.assistant").id("msg.1").child(
                    Node::new(
                        "message.assistant.thinking",
                        Kind::Collapsible {
                            summary: vec![
                                Span::strong("thinking"),
                                Span::plain(" · one\ntwo\nthree\n… 3 lines hidden"),
                            ],
                        },
                    )
                    .id("msg.1.thinking")
                    .child(Node::text(
                        "message.assistant.thinking.text",
                        [Span::plain("one\ntwo\nthree\nfour\nfive\nsix")],
                    )),
                ),
            ),
        );
        let retained = Retained::new(view, &screen);
        let lines = all(&retained);
        let rows = lines.iter().map(Line::text).collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![
                "┃ ",
                "┃ ",
                "┃ thinking · one",
                "┃ two",
                "┃ three",
                "┃ … 3 lines hidden",
                "┃ ",
                "",
                "┃ ",
            ]
        );
        assert_eq!(
            lines[1].spans[0].0,
            screen.theme.role("message.assistant.thinking.rail")
        );
    }

    #[test]
    fn a_canonical_panel_is_exclusive_and_does_not_become_transcript_content() {
        let screen = Screen::new(40, 12);
        let view = Node::section("session")
            .id("session")
            .child(
                Node::section("transcript")
                    .id("transcript")
                    .child(text("body", "conversation")),
            )
            .child(
                Node::section("panel")
                    .id("login")
                    .label("Credential")
                    .action(misa_proto::view::Action {
                        id: "panel.close".into(),
                        on: misa_proto::view::ActionOn::Click,
                        label: Some("Close".into()),
                        args: misa_value::Value::Null,
                    })
                    .child(Node::text("panel.copy", [Span::plain("enter a token")])),
            );
        let mut retained = Retained::new(view, &screen);
        let frame = retained.frame(&screen, None);
        let rows = frame.lines.iter().map(Line::text).collect::<Vec<_>>();
        assert!(rows.iter().any(|row| row.contains("Credential")));
        assert!(rows.iter().any(|row| row.contains("enter a token")));
        assert!(rows.iter().any(|row| row.contains("esc Close")), "{rows:?}");
        assert!(
            rows.iter().position(|row| row.contains("Credential"))
                > rows.iter().position(|row| row.contains("misa"))
        );
        assert!(
            !retained
                .placed_lines(64)
                .0
                .iter()
                .any(|line| line.text().contains("Credential"))
        );
    }
    #[test]
    fn append_formatting_and_viewport_copy_work_do_not_grow_with_history() {
        let mut results = vec![];
        let mut baselines = vec![];
        for history in [100, 1000] {
            for _ in 0..6 {
                let screen = Screen::new(40, 16);
                let mut retained = Retained::new(document(history), &screen);
                retained.current(
                    Stream {
                        id: "msg9999.body".into(),
                        role: "assistant".into(),
                        text: "current ".into(),
                    },
                    &screen,
                );
                retained.reindex();
                retained.work = Work::default();
                let mut offset = "current ".len();
                let mut baseline_visits = 0;
                let mut baseline_output = String::new();
                for text in ["one ", "two ", "界🙂", "\n", "final"] {
                    retained
                        .observed(
                            &misa_client::document::Update::Changed {
                                member: "conversation".into(),
                                applied: std::sync::Arc::new(
                                    misa_protocol::observation::Applied::Changed(
                                        std::collections::BTreeMap::from([(
                                            "conversation".into(),
                                            misa_protocol::observation::MemberChange::Document {
                                                tree: vec![],
                                                live: vec![StreamUpdate::Append {
                                                    id: "msg9999.body".into(),
                                                    offset,
                                                    text: text.into(),
                                                }],
                                                reset_live: false,
                                            },
                                        )]),
                                    ),
                                ),
                            },
                            &screen,
                        )
                        .unwrap();
                    offset += text.len();
                    let rendered = retained.draw(&screen);
                    assert!(
                        rendered
                            .iter()
                            .any(|line| line.node.as_deref() == Some("msg9999.body"))
                    );
                    // The previous full-tree presentation path remains callable as
                    // a baseline. Count its actual recursive resolve visits.
                    let legacy = retained.interaction();
                    crate::RESOLVE_VISITS.with(|visits| visits.set(0));
                    baseline_output.push_str(&misa_lines::to_plain(&crate::draw(&screen, &legacy)));
                    baseline_visits += crate::RESOLVE_VISITS.with(|visits| visits.get());
                }
                assert_eq!(
                    retained.live["msg9999.body"].stream.text,
                    "current one two 界🙂\nfinal"
                );
                assert_eq!(retained.work.formatted_nodes, 0);
                assert_eq!(retained.work.appended_bytes, 21);
                results.push(retained.work);
                baselines.push((baseline_visits, baseline_output));
            }
        }
        assert!(
            results.iter().all(|work| (
                work.formatted_nodes,
                work.appended_bytes,
                work.copied_rows
            ) == (
                results[0].formatted_nodes,
                results[0].appended_bytes,
                results[0].copied_rows
            )),
            "{results:?}"
        );
        assert!(results.iter().all(|work| work.index_steps <= 5 * 24));
        assert!(baselines[..6].iter().all(|run| run == &baselines[0]));
        assert!(baselines[6..].iter().all(|run| run == &baselines[6]));
        eprintln!(
            "baseline actual resolve visits: 100-history {}, 1000-history {}; one output per size across six runs",
            baselines[0].0, baselines[6].0
        );
        eprintln!("100/1000 history, six runs each: {results:?}");
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::{Key, KeyOut};
    use misa_proto::view::Span;
    #[test]
    fn live_selection_copies_the_displayed_wrapped_row() {
        let mut screen = Screen::new(10, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.body".into(),
                role: "assistant".into(),
                text: "hello world".into(),
            },
            &screen,
        );
        retained.reindex();
        // A live row wraps at the word, exactly as the settled block will.
        assert_eq!(
            retained.live["msg1.body"]
                .lines
                .iter()
                .map(Line::text)
                .collect::<Vec<_>>(),
            ["hello", "world"]
        );
        screen.key(Key::Escape);
        assert_eq!(
            retained.selection_key(&mut screen, &Key::Char('v')),
            Some(KeyOut::Local)
        );
        retained.selection_key(&mut screen, &Key::Motion(crate::ed::Motion::LineEnd));
        assert_eq!(
            retained.selection_key(&mut screen, &Key::Char('y')),
            Some(KeyOut::Copy("world".into()))
        );
    }
    #[test]
    fn full_viewport_reserves_staged_attachment_and_multiline_editor_rows() {
        let mut root = Node::section("session").id("session");
        for i in 0..100 {
            root.children
                .push(Node::text("text", [Span::plain("history")]).id(format!("n{i}")));
        }
        let mut screen = Screen::new(30, 12);
        screen.editor.set_text("first\nsecond");
        let mut retained = Retained::new(root, &screen);
        let frame = retained.frame(&screen, Some("1 clipboard attachments · 1 uploading"));
        assert_eq!(frame.lines.len(), 12);
        assert!(
            frame
                .lines
                .iter()
                .all(|line| !line.text().contains('\n') && misa_render::width(&line.text()) <= 30)
        );
        assert_eq!(frame.cursor_row, 11);
        assert_eq!(frame.cursor_column, 8);
    }
    #[test]
    fn folded_content_edits_and_trailing_blanks_match_the_settled_renderer() {
        let mut screen = Screen::new(40, 40);
        let folded = Node::new(
            "call",
            Kind::Collapsible {
                summary: vec![Span::plain("summary")],
            },
        )
        .id("call")
        .child(
            Node::new("quote", Kind::Quote)
                .id("quote")
                .child(Node::text("text", [Span::plain("before")]).id("body")),
        );
        let root = Node::section("session")
            .id("session")
            .child(folded)
            .child(Node::text("text", [Span::plain("")]).id("blank"));
        let mut retained = Retained::new(root, &screen);
        retained
            .op(
                &ViewOp::Replace {
                    id: "body".into(),
                    node: Node::text("text", [Span::plain("after")]).id("body"),
                },
                &screen,
            )
            .unwrap();
        retained.reindex();
        screen.prefs.open_all();
        retained.local(&screen);
        let expected = misa_lines::render(
            &screen.resolve(&retained.tree.snapshot()),
            &screen.theme,
            40,
        );
        let frame = retained.frame(&screen, None);
        let top = crate::chrome::top(&screen, 0, None);
        assert_eq!(
            &frame.lines[top.len()..top.len() + expected.len()],
            expected.as_slice()
        );
        // The transcript, the blank separator, and then the composer.
        assert_eq!(
            frame.lines.len(),
            expected.len() + top.len() + 1 + crate::chrome::composer(&screen).lines.len()
        );
    }
}
