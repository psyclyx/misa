//! Canonical document index: node ownership, stream rows and layout arithmetic.
use super::{Screen, Work};
use misa_linear::{Live, Paint, THINKING_TAIL_LINES, stream_order, thinking_stream};
use misa_lines::Line;
use misa_proto::Node;
use misa_proto::sync::{IndexedTree, Stream, StreamUpdate, ViewOp};
use misa_proto::view::{ActionOn, Kind};
use std::collections::HashMap;

/// Rows an image may occupy in the current view. The verbose transcript lets an
/// image use more of the viewport; otherwise it stays a compact thumbnail so a
/// single screenshot cannot push the whole conversation off screen.
pub(super) fn image_max_rows(screen: &Screen) -> u16 {
    if screen.prefs.is_open("*") {
        misa_terminal_ui::graphics::VERBOSE_ROWS
    } else {
        misa_terminal_ui::graphics::COMPACT_ROWS
    }
}

/// The native pixel dimensions of every image node in a resolved subtree.
fn collect_image_dims(node: &Node, out: &mut HashMap<String, (u32, u32)>) {
    if let Kind::Image { width, height, .. } = &node.kind {
        out.insert(node.id.clone(), (*width, *height));
    }
    for child in &node.children {
        collect_image_dims(child, out);
    }
}

/// Replace each rendered image placeholder with the rows its placement needs.
///
/// The linear renderer produces one row for a `Kind::Image`. The terminal knows
/// the cell size and the column budget, so it owns how many rows the placement
/// actually consumes; the first row keeps the alt/dimension label and the rest
/// are blank rows the image will cover.
fn reserve_image_rows(node: &Node, lines: Vec<Line>, screen: &Screen) -> Vec<Line> {
    if !screen.graphics.enabled() {
        return lines;
    }
    let mut images = HashMap::new();
    collect_image_dims(node, &mut images);
    if images.is_empty() {
        return lines;
    }
    let max_rows = image_max_rows(screen);
    let mut expanded = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let Some(id) = line.node.as_deref() else {
            out.push(line);
            continue;
        };
        let Some(&(width, height)) = images.get(id) else {
            out.push(line);
            continue;
        };
        // A component may draw more than one row for one image node; only the
        // first anchors a placement, so only the first reserves rows.
        if !expanded.insert(id.to_string()) {
            out.push(line);
            continue;
        }
        let plan = screen.graphics.plan(
            width,
            height,
            screen.width.saturating_sub(line.indent as u16),
            max_rows,
        );
        out.push(line.clone());
        for _ in 1..plan.rows {
            out.push(Line {
                indent: line.indent,
                surface: line.surface,
                node: Some(id.to_string()),
                spans: Vec::new(),
            });
        }
    }
    out
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

#[derive(Clone)]
enum Segment {
    Owner(String),
    Live(String),
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
pub(super) struct Mutation {
    pub layout_changed: bool,
    pub rebuilt: bool,
    pub work: Work,
}
pub(super) struct DocumentIndex {
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
    width: u16,
    theme: String,
    opened: Vec<String>,
    components: misa_lines::components::Settings,
}
impl DocumentIndex {
    pub(super) fn new(view: Node, screen: &Screen) -> (Self, Work) {
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
            width: screen.width,
            theme: screen.theme.name.clone(),
            opened: screen.prefs.opened.clone(),
            components: screen.prefs.components.clone(),
        };
        let mut work = Work::default();
        out.order = out.build(view, 0, 0, screen, &mut work);
        out.reindex();
        (out, work)
    }
    pub(super) fn interaction(&self) -> Node {
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
    pub(super) fn has_turn(&self) -> bool {
        self.tree.contains("turn")
    }
    /// The blob references the current tree names. The client fetches these off
    /// the render path; until the pixels are cached the image keeps its placeholder.
    pub(super) fn image_blobs(&self) -> Vec<misa_proto::view::BlobRef> {
        self.tree
            .nodes()
            .filter_map(|node| match &node.kind {
                Kind::Image { blob, .. } => Some(blob.clone()),
                _ => None,
            })
            .collect()
    }
    fn build(
        &mut self,
        mut node: Node,
        depth: usize,
        level: usize,
        screen: &Screen,
        work: &mut Work,
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
        work.formatted_nodes += members.len();
        for member in &members {
            self.membership.insert(member.clone(), id.clone());
        }
        let child_depth = depth + usize::from(screen.theme.rail(&node.role).is_some());
        let lines = component.unwrap_or_else(|| {
            misa_lines::render_block(&resolved, &screen.theme, screen.width as usize, depth)
        });
        // The linear renderer degrades an image to one placeholder row. The
        // terminal knows the placement height, so reserve the extra rows here: the
        // viewport's row arithmetic then accounts for the space the image needs.
        let lines = reserve_image_rows(&resolved, lines, screen);
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
            order.extend(self.build(child, child_depth, level + 1, screen, work));
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
    fn refresh(&mut self, id: &str, screen: &Screen, work: &mut Work) {
        let range = self.range(id);
        let at = range.start;
        let owner = &self.owners[id];
        let (depth, level) = (owner.depth, owner.level);
        self.erase(range);
        if let Some(node) = self.tree.subtree(id) {
            let added = self.build(node, depth, level, screen, work);
            self.order.splice(at..at, added);
        }
    }
    fn op(&mut self, op: &ViewOp, screen: &Screen, work: &mut Work) -> Result<(), String> {
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
            self.refresh(&owner, screen, work);
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
                let added = self.build(node.clone(), depth, level, screen, work);
                self.order.splice(at..at, added);
            }
            ViewOp::Remove { id } => {
                let range = self.range(id);
                self.tree.apply(op)?;
                self.erase(range);
            }
            ViewOp::Replace { id, .. } => {
                self.tree.apply(op)?;
                self.refresh(id, screen, work);
            }
        }
        Ok(())
    }
    fn reindex(&mut self) {
        self.segments.clear();
        self.live_positions.clear();
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
                .rposition(|line| self.line_occupied(line))
                .map_or(0, |at| at + 1);
            self.lengths[index] = trimmed;
            if trimmed > 0 {
                break;
            }
        }
        self.rows = Rows::new(&self.lengths);
    }
    fn lines(&self, segment: &Segment) -> &[Line] {
        match segment {
            Segment::Owner(id) => &self.owners[id].lines,
            Segment::Live(id) => &self.live[id].lines,
        }
    }

    /// Whether a row holds something that must survive trailing-whitespace
    /// trimming. A reserved image row is blank but occupied: the terminal draws
    /// the image over it, so trimming it would let the image overrun what follows.
    fn line_occupied(&self, line: &Line) -> bool {
        !line.is_blank()
            || line.node.as_deref().is_some_and(|id| {
                self.tree
                    .node(id)
                    .is_some_and(|node| matches!(&node.kind, Kind::Image { .. }))
            })
    }

    /// The line at a physical row, or `None` for a row outside the document.
    pub(super) fn row_line(&self, row: usize) -> Option<&Line> {
        let (index, offset, _) = self.rows.locate(row);
        if index >= self.segments.len() {
            return None;
        }
        self.lines(&self.segments[index])[..self.lengths[index]].get(offset)
    }

    pub(super) fn current(&mut self, stream: Stream, screen: &Screen) {
        let id = stream.id.clone();
        let tail = (thinking_stream(&stream.role) && !screen.prefs.is_open(&id))
            .then_some(THINKING_TAIL_LINES);
        let paint = Paint::of(&screen.theme, screen.width, &stream.role);
        let mut live = Live::of(
            Stream {
                text: String::new(),
                ..stream.clone()
            },
            tail,
        );
        live.append(&stream.text, &paint);
        self.live.insert(id, live);
    }
    /// Apply protocol view operations from a local document owner. The remote
    /// replica still owns sequencing/recovery for network transactions.
    pub(super) fn apply_ops(
        &mut self,
        ops: &[ViewOp],
        screen: &Screen,
    ) -> Result<Mutation, String> {
        let mut work = Work::default();
        for op in ops {
            self.op(op, screen, &mut work)?;
        }
        self.reindex();
        Ok(Mutation {
            layout_changed: true,
            rebuilt: false,
            work,
        })
    }

    /// Apply a transaction translated by the connected host, without losing
    /// incremental stream append accounting or the canonical tree index.
    pub(super) fn changed(
        &mut self,
        tree: &[ViewOp],
        live: &[StreamUpdate],
        reset_live: bool,
        screen: &Screen,
    ) -> Result<Mutation, String> {
        let mut work = Work::default();
        let mut layout_changed = false;
        let mut structural = !tree.is_empty() || reset_live;
        for op in tree {
            self.op(op, screen, &mut work)?;
        }
        if reset_live {
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
                    let role = self
                        .live
                        .get(id)
                        .ok_or("Missing rendered stream")?
                        .stream
                        .role
                        .clone();
                    let paint = Paint::of(&screen.theme, screen.width, &role);
                    let live = self.live.get_mut(id).ok_or("Missing rendered stream")?;
                    if live.stream.text.len() != *offset {
                        return Err("Rendered stream offset gap".into());
                    }
                    live.append(text, &paint);
                    work.appended_bytes += text.len();
                    if !structural {
                        if let Some(&index) = self.live_positions.get(id) {
                            let new = if self.rows.total() == self.rows.prefix(index + 1) {
                                live.last_nonblank
                            } else {
                                live.lines.len()
                            };
                            work.index_steps += self.rows.change(index, self.lengths[index], new);
                            self.lengths[index] = new;
                            layout_changed = true;
                        }
                    }
                }
            }
        }
        if structural {
            self.reindex();
            layout_changed = true;
        }
        Ok(Mutation {
            layout_changed,
            rebuilt: false,
            work,
        })
    }
    pub(super) fn reset(&mut self, tree: Node, streams: &[Stream], screen: &Screen) -> Mutation {
        let (new, work) = Self::new(tree, screen);
        *self = new;
        for stream in streams {
            self.current(stream.clone(), screen);
        }
        self.reindex();
        Mutation {
            layout_changed: true,
            rebuilt: true,
            work,
        }
    }
    pub(super) fn local(&mut self, screen: &Screen) -> Mutation {
        let mut work = Work::default();
        let mut layout_changed = false;
        let mut rebuilt = false;
        if self.width != screen.width
            || self.theme != screen.theme.name
            || self.opened != screen.prefs.opened
            || self.components != screen.prefs.components
        {
            let streams: Vec<_> = self.live.values().map(|live| live.stream.clone()).collect();
            let (new, built) = Self::new(self.tree.snapshot(), screen);
            work += built;
            rebuilt = true;
            *self = new;
            for stream in streams {
                self.current(stream, screen);
            }
            self.reindex();
            layout_changed = true;
        } else if let Some(panel) = self.panel.clone() {
            if let Some(id) = self.membership.get(&panel).cloned() {
                self.refresh(&id, screen, &mut work);
                self.reindex();
                layout_changed = true;
            }
        }
        Mutation {
            layout_changed,
            rebuilt,
            work,
        }
    }
    /// Independent documents share placement and components, never node identity.
    pub(super) fn placed_lines(&self, limit: usize) -> (Vec<Line>, Vec<Line>) {
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
    pub(super) fn surface_lines(&self, screen: &Screen) -> Vec<Line> {
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
    pub(super) fn image_hash(&self, id: &str) -> Option<&str> {
        match &self.tree.node(id)?.kind {
            Kind::Image { blob, .. } => Some(&blob.hash),
            _ => None,
        }
    }
    pub(super) fn total_rows(&self) -> usize {
        self.rows.total()
    }
    pub(super) fn row_node(&self, row: usize) -> Option<String> {
        self.row_line(row).and_then(|line| line.node.clone())
    }
    pub(super) fn node_row(&self, key: &str, offset: usize) -> Option<usize> {
        let mut row = 0;
        for (index, segment) in self.segments.iter().enumerate() {
            for line in &self.lines(segment)[..self.lengths[index]] {
                if line.node.as_deref() == Some(key) {
                    return Some(row + offset);
                }
                row += 1;
            }
        }
        None
    }
    pub(super) fn visible_rows(&self, start: usize, room: usize) -> (Vec<Line>, usize) {
        let (mut segment, mut offset, steps) = self.rows.locate(start);
        let mut lines = Vec::new();
        while lines.len() < room && segment < self.segments.len() {
            let rows = &self.lines(&self.segments[segment])[..self.lengths[segment]];
            let take = (room - lines.len()).min(rows.len().saturating_sub(offset));
            lines.extend_from_slice(&rows[offset..offset + take]);
            segment += 1;
            offset = 0;
        }
        (lines, steps)
    }
    pub(super) fn selection_lines(&self) -> Vec<Line> {
        self.segments
            .iter()
            .enumerate()
            .flat_map(|(i, segment)| self.lines(segment)[..self.lengths[i]].iter().cloned())
            .collect()
    }
    pub(super) fn footer_lines(&self, queue: bool) -> impl Iterator<Item = Line> + '_ {
        self.order
            .iter()
            .filter_map(|id| self.owners.get(id))
            .filter(move |owner| owner.footer && (owner.role == "queue") == queue)
            .flat_map(|owner| owner.lines.iter().cloned())
    }
    pub(super) fn has_panel(&self) -> bool {
        self.panel.is_some()
    }
    #[cfg(test)]
    pub(super) fn stream_text(&self, id: &str) -> &str {
        &self.live[id].stream.text
    }
    #[cfg(test)]
    pub(super) fn stream_count(&self) -> usize {
        self.live.len()
    }
    #[cfg(test)]
    pub(super) fn append_unindexed(&mut self, id: &str, text: &str, paint: &Paint) {
        self.live.get_mut(id).unwrap().append(text, paint);
    }
    #[cfg(test)]
    pub(super) fn stream_lines(&self, id: &str) -> &[Line] {
        &self.live[id].lines
    }
    pub(super) fn attachments(&self) -> usize {
        self.attachments
    }
    #[cfg(test)]
    pub(super) fn snapshot(&self) -> Node {
        self.tree.snapshot()
    }
    #[cfg(test)]
    pub(super) fn contains(&self, id: &str) -> bool {
        self.tree.contains(id)
    }
    #[cfg(test)]
    pub(super) fn all_lines(&self) -> Vec<Line> {
        self.segments
            .iter()
            .flat_map(|segment| self.lines(segment).iter().cloned())
            .collect()
    }
}
