//! Client-owned interaction and layout. No field draft, disclosure state or destination leaves
//! this module until the person activates an action the session advertised.
use crate::{Layout, Op, Scene};
use misa_kit::editor::{Editor, Motion};
use misa_proto::sync::{IndexedTree, StreamUpdate, ViewOp};
use misa_proto::view::{ActionOn, FieldKind, Kind, Node};
use misa_proto::wire::Intent;
use misa_render::{Style, Theme};
#[cfg(test)]
use misa_render::Color;
use misa_value::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Appearance(crate::appearance::Choice),
    InvokeInstalled {
        command: String,
        input: Value,
    },
    Intent(Intent),
    Save {
        node: String,
        destination: String,
    },
    LoadImage(misa_proto::view::BlobRef),
    Connect(String),
    DaemonInvoke {
        daemon: String,
        command: String,
        input: Value,
    },
    Archive {
        daemon: String,
        prefix: String,
    },
    Discover,
    Presentation {
        id: String,
        choice: misa_client::composition::Choice,
    },
    Disconnect(String),
    CloseInstance {
        daemon: String,
        scope: misa_proto::observation::Scope,
    },
    Select {
        daemon: String,
        scope: misa_proto::observation::Scope,
    },
    Request {
        id: String,
        generation: i64,
        action: String,
        fields: BTreeMap<String, Value>,
    },
    Copy(String),
    Form {
        action: String,
        drafts: BTreeMap<String, String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    Field { node: String, field: String },
    Action { node: String, action: String },
    Disclosure(String),
    SavePath,
    SaveConfirm,
    SaveCancel,
    Text(usize),
    LoadImage(misa_proto::view::BlobRef),
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub control: Control,
}
impl Hit {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}
#[derive(Clone, Debug)]
struct TextRow {
    x: f32,
    y: f32,
    text: String,
}
#[derive(Clone)]
struct Cached {
    width: f32,
    height: f32,
    ops: Arc<Vec<Op>>,
    hits: Vec<Hit>,
    rows: Vec<TextRow>,
}
#[derive(Clone, Debug)]
pub enum Key {
    Text(String),
    Commands,
    Up,
    Down,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Enter { newline: bool },
    Tab { backward: bool },
    Escape,
    Copy,
    SelectAll,
}

pub struct App {
    light: bool,
    #[cfg(test)]
    rendered_nodes: usize,
    tree: IndexedTree,
    root: String,
    streams: BTreeMap<String, Node>,
    cache: BTreeMap<String, Arc<Cached>>,
    cache_width: u32,
    pub commands: Vec<misa_kit::intent::Command>,
    pub notice: String,
    pub hits: Vec<Hit>,
    pub focus: Option<Control>,
    pub expanded: BTreeSet<String>,
    pub images: BTreeMap<String, Arc<image::RgbaImage>>,
    drafts: BTreeMap<(String, String), Editor>,
    save: Option<(String, Editor)>,
    picker: Option<misa_kit::picker::Picker>,
    report: Option<Report>,
    selection: Option<((usize, usize), (usize, usize))>,
    rows: Vec<TextRow>,
    replace_selection: bool,
    scroll: f32,
    follow: bool,
    content_height: f32,
    viewport_height: f32,
}
struct Report {
    title: String,
    entries: Vec<String>,
    offset: usize,
    columns: usize,
    lines: Vec<String>,
}
impl Report {
    fn collect(value: &Value, path: &str, entries: &mut Vec<String>) {
        match value {
            Value::Map(fields) if !fields.is_empty() => {
                for (key, value) in fields.iter() {
                    let label = key.replace('_', " ");
                    let path = if path.is_empty() {
                        label
                    } else {
                        format!("{path} / {label}")
                    };
                    Self::collect(value, &path, entries);
                }
            }
            Value::List(values) if !values.is_empty() => {
                for (index, value) in values.iter().enumerate() {
                    Self::collect(value, &format!("{path} / {}", index + 1), entries);
                }
            }
            _ => {
                let content = match value {
                    Value::Null => "Unavailable".into(),
                    Value::Map(_) | Value::List(_) => "None".into(),
                    Value::Bytes(bytes) => format!("{} bytes", bytes.len()),
                    _ => misa_render::fact::format("value.text", value),
                };
                entries.push(if path.is_empty() {
                    content
                } else {
                    format!("{path}: {content}")
                });
            }
        }
    }
}
impl App {
    pub fn new(view: Node) -> Self {
        let mut app = Self {
            #[cfg(test)]
            rendered_nodes: 0,
            tree: IndexedTree::new(Node::section("session")),
            root: "session".into(),
            streams: BTreeMap::new(),
            cache: BTreeMap::new(),
            cache_width: 0,
            light: false,
            commands: vec![],
            notice: String::new(),
            hits: vec![],
            focus: None,
            expanded: BTreeSet::new(),
            images: BTreeMap::new(),
            drafts: BTreeMap::new(),
            save: None,
            picker: None,
            report: None,
            selection: None,
            rows: vec![],
            replace_selection: false,
            scroll: 0.0,
            follow: true,
            content_height: 0.0,
            viewport_height: 600.0,
        };
        app.set_view(view);
        app
    }
    pub fn set_light(&mut self, light: bool) {
        if self.light != light {
            self.light = light;
            self.cache.clear();
        }
    }
    fn colors(&self) -> crate::appearance::Palette {
        crate::appearance::Palette::new(self.light)
    }
    pub fn report(&mut self, title: String, value: Value) {
        let mut entries = Vec::new();
        Report::collect(&value, "", &mut entries);
        self.picker = None;
        self.report = Some(Report {
            title,
            entries,
            offset: 0,
            columns: 0,
            lines: vec![],
        });
    }
    pub fn clear_secret_drafts(&mut self) {
        for ((node, field), edit) in &mut self.drafts {
            if self.tree.node(node).is_some_and(|node| matches!(&node.kind, Kind::Fields {fields} if fields.iter().any(|value| &value.id == field && value.secret))) { edit.set_text(""); }
        }
        self.cache.clear();
    }
    pub fn reject_prompt(&mut self, text: String, reason: String) {
        let target = self
            .drafts
            .keys()
            .find(|(_, field)| field == "prompt")
            .cloned();
        if let Some(key) = target.filter(|key| self.drafts[key].text().is_empty()) {
            self.drafts.get_mut(&key).unwrap().set_text(text);
            self.invalidate(&key.0);
        } else {
            // Keep newer typing intact. The separate local report can be copied
            // even if the original composer was removed by a scope update.
            self.report("Unsent prompt · Copy to recover".into(), Value::str(text));
        }
        self.notice = reason;
    }
    pub fn set_view(&mut self, mut view: Node) {
        misa_proto::sync::address(&mut view);
        self.cache.clear();
        self.streams.clear();
        let mut references = BTreeSet::new();
        image_hashes(&view, &mut references);
        self.images.retain(|hash, _| references.contains(hash));
        fn fields(node: &Node, keys: &mut Vec<(String, misa_proto::view::Field)>) {
            if let Kind::Fields { fields } = &node.kind {
                for field in fields {
                    if !field.read_only {
                        keys.push((node.id.clone(), field.clone()));
                    }
                }
            }
            for child in &node.children {
                fields(child, keys);
            }
            if let Kind::List { items, .. } = &node.kind {
                for child in items.iter().flatten() {
                    fields(child, keys);
                }
            }
        }
        let mut keys = vec![];
        fields(&view, &mut keys);
        self.drafts.retain(|(node, field), _| {
            keys.iter()
                .any(|(id, value)| id == node && &value.id == field)
        });
        for (node, field) in &keys {
            self.drafts
                .entry((node.clone(), field.id.clone()))
                .or_insert_with(|| {
                    let mut edit = Editor::new();
                    let value = match &field.kind {
                        FieldKind::Choice {
                            selected: Some(value),
                            ..
                        } => value,
                        _ => &field.value,
                    };
                    edit.set_text(value);
                    edit
                });
        }
        if self.save.is_some() {
            // A live update cannot steal focus from a local destination dialog.
        } else if let Some((node, field)) = keys.iter().find(|(node, _)| node == "panel.input" && self.tree.node(node).is_none()) {
            if !matches!(&self.focus, Some(Control::Field { node: focused, .. }) if focused == node) {
                self.focus = Some(Control::Field { node: node.clone(), field: field.id.clone() });
            }
        } else if self.focus.as_ref().is_none_or(|control| matches!(control, Control::Field { node, field } if !self.drafts.contains_key(&(node.clone(),field.clone())))) {
            self.focus = keys.last().map(|(node,field)| Control::Field { node: node.clone(), field: field.id.clone() });
        }
        self.root = view.id.clone();
        self.tree = IndexedTree::new(view);
        self.selection = None;
    }

    fn visible_streams(&self) -> Vec<String> {
        self.streams.iter().filter(|(id,node)| {
            !self.tree.contains(id.rsplit_once('.').map(|(owner,_)|owner).unwrap_or(id)) && matches!(&node.kind,Kind::Text {spans} if spans.iter().any(|span|!span.text.is_empty()))
        }).map(|(id,_)|id.clone()).collect()
    }
    fn stream_parent(&self) -> &str {
        if self.tree.contains("transcript") {
            "transcript"
        } else {
            &self.root
        }
    }
    fn invalidate(&mut self, id: &str) {
        let mut cursor = Some(id.to_string());
        while let Some(id) = cursor {
            self.cache.remove(&id);
            cursor = self.tree.parent(&id).map(str::to_owned);
        }
    }
    fn invalidate_focus(&mut self) {
        let id = match &self.focus {
            Some(Control::Field { node, .. })
            | Some(Control::Action { node, .. })
            | Some(Control::Disclosure(node)) => Some(node.clone()),
            _ => None,
        };
        if let Some(id) = id {
            self.invalidate(&id);
        }
    }
    fn invalidate_streams(&mut self) {
        self.cache.remove("streams");
        let parent = self.stream_parent().to_string();
        self.invalidate(&parent);
    }
    fn forget_subtree(&mut self, id: &str) {
        if let Some(node) = self.tree.node(id) {
            let mut references = BTreeSet::new();
            image_hashes(node, &mut references);
            let mut removed = false;
            for hash in references {
                removed |= self.images.remove(&hash).is_some();
            }
            if removed {
                self.cache.clear();
            }
        }
        for child in self.tree.children(id) {
            self.forget_subtree(&child);
        }
        self.cache.remove(id);
    }
    fn refresh_fields(&mut self, node: &Node) {
        if let Kind::Fields { fields } = &node.kind {
            for field in fields.iter().filter(|field| !field.read_only) {
                self.drafts
                    .entry((node.id.clone(), field.id.clone()))
                    .or_insert_with(|| {
                        let mut edit = Editor::new();
                        let value = match &field.kind {
                            FieldKind::Choice {
                                selected: Some(value),
                                ..
                            } => value,
                            _ => &field.value,
                        };
                        edit.set_text(value);
                        edit
                    });
            }
            if node.id == "panel.input" && self.save.is_none() {
                if let Some(field) = fields.iter().find(|field| !field.read_only) {
                    self.focus = Some(Control::Field {
                        node: node.id.clone(),
                        field: field.id.clone(),
                    });
                }
            }
        }
        for child in &node.children {
            self.refresh_fields(child);
        }
    }
    fn apply_tree<'a>(
        &mut self,
        operations: impl IntoIterator<Item = &'a ViewOp>,
    ) -> Result<(), String> {
        for op in operations {
            match op {
                ViewOp::Insert { parent, node, .. } => {
                    self.invalidate(parent);
                    self.tree.apply(op)?;
                    self.refresh_fields(node);
                }
                ViewOp::Remove { id } | ViewOp::Replace { id, .. } => {
                    self.invalidate(id);
                    self.forget_subtree(id);
                    self.tree.apply(op)?;
                    if let ViewOp::Replace { node, .. } = op {
                        self.refresh_fields(node);
                    }
                }
            }
        }
        self.drafts.retain(|(id,field),_|self.tree.node(id).is_some_and(|node|matches!(&node.kind,Kind::Fields {fields} if fields.iter().any(|value|&value.id==field && !value.read_only))));
        if matches!(&self.focus,Some(Control::Field {node,field}) if !self.drafts.contains_key(&(node.clone(),field.clone())))
        {
            self.focus = None;
        }
        if self.focus.is_none() && self.save.is_none() {
            self.focus = self
                .drafts
                .keys()
                .next_back()
                .map(|(node, field)| Control::Field {
                    node: node.clone(),
                    field: field.clone(),
                });
        }
        Ok(())
    }
    fn reset_streams(&mut self, streams: &[misa_proto::sync::Stream]) {
        for id in self.streams.keys() {
            self.cache.remove(id);
        }
        self.streams.clear();
        for stream in streams {
            self.streams.insert(
                stream.id.clone(),
                Node::text(&stream.role, [misa_proto::view::Span::plain(&stream.text)])
                    .id(&stream.id)
                    .state(misa_proto::view::State::Streaming),
            );
        }
        self.invalidate_streams();
    }
    fn apply_stream(&mut self, update: &StreamUpdate) {
        match update {
            StreamUpdate::Current { stream } => {
                self.cache.remove(&stream.id);
                self.streams.insert(
                    stream.id.clone(),
                    Node::text(&stream.role, [misa_proto::view::Span::plain(&stream.text)])
                        .id(&stream.id)
                        .state(misa_proto::view::State::Streaming),
                );
            }
            StreamUpdate::Append { id, text, .. } => {
                self.cache.remove(id);
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
                self.cache.remove(id);
                self.streams.remove(id);
            }
        }
        self.invalidate_streams();
    }
    /// One already-validated replica transaction. The window paints only after
    /// canonical changes and live retirement have both reached its derived cache.
    pub fn observed(&mut self, update: &misa_client::document::Update) -> Result<(), String> {
        use misa_client::document::Update;
        use misa_protocol::observation::{Applied, MemberChange, Status};
        match update {
            Update::Reset(document) => {
                self.set_view(document.tree.clone());
                self.reset_streams(&document.streams);
            }
            Update::Changed { member, applied } => {
                if let Applied::Changed(members) = applied.as_ref() {
                    if let Some(MemberChange::Document {
                        tree,
                        live,
                        reset_live,
                    }) = members.get(member)
                    {
                        if !tree.is_empty() {
                            self.apply_tree(tree)?;
                        }
                        if *reset_live {
                            self.reset_streams(&[]);
                        }
                        for update in live {
                            self.apply_stream(update);
                        }
                    }
                }
            }
            Update::Unavailable(fault) => self.notice = fault.message.clone(),
            Update::Status(status) => {
                self.notice = match status {
                    Status::Awaiting => "Loading session…".into(),
                    Status::Current => String::new(),
                    Status::Recovering(_) => "Refreshing session…".into(),
                    Status::Stale(fault) | Status::Closed(fault) => fault.message.clone(),
                }
            }
        }
        Ok(())
    }
    pub fn receive(&mut self, message: &misa_proto::SessionMsg) -> Result<(), String> {
        use misa_proto::{SessionEvent, SessionMsg};
        match message {
            SessionMsg::View { view, .. } => self.set_view(view.clone()),
            SessionMsg::Changes { changes, .. } => {
                self.apply_tree(changes.iter().flat_map(|change| &change.ops))?
            }
            SessionMsg::Streams { streams } => self.reset_streams(streams),
            SessionMsg::Event {
                event: SessionEvent::Stream { update },
                ..
            } => {
                self.apply_stream(update);
            }
            _ => {}
        }
        Ok(())
    }
    pub fn image(&mut self, hash: String, image: Arc<image::RgbaImage>) {
        let mut live = BTreeSet::new();
        let mut pending = vec![self.root.clone()];
        while let Some(id) = pending.pop() {
            if let Some(node) = self.tree.node(&id) {
                image_hashes(node, &mut live);
            }
            pending.extend(self.tree.children(&id));
        }
        if !live.contains(&hash) {
            return;
        }
        const MAX_BYTES: usize = 32 * 1024 * 1024;
        let bytes = image.as_raw().len();
        if bytes > MAX_BYTES {
            self.notice = "Image exceeds the decoded cache limit".into();
            return;
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
        // Cached display lists own image Arcs too. Eviction must release those
        // copies and expose the local Load image affordance on remaining owners.
        if evicted {
            self.cache.clear();
        }
        // Image decode is infrequent; only owners containing this reference are invalidated.
        let ids: Vec<_> = self
            .cache
            .keys()
            .filter(|id| {
                self.tree.node(id).is_some_and(|node| {
                    let mut hashes = BTreeSet::new();
                    image_hashes(node, &mut hashes);
                    hashes.contains(&hash)
                })
            })
            .cloned()
            .collect();
        for id in ids {
            self.invalidate(&id);
        }
        self.images.insert(hash, image);
    }
    fn present(
        &mut self,
        id: &str,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        let cached = if let Some(cached) = self.cache.get(id).filter(|cached| cached.width == width)
        {
            cached.clone()
        } else {
            let node = if id == "streams" {
                Node::section("streams").id("streams")
            } else if let Some(node) = self.streams.get(id).or_else(|| self.tree.node(id)) {
                node.clone()
            } else {
                return;
            };
            let outer_hits = std::mem::take(&mut self.hits);
            let outer_rows = std::mem::take(&mut self.rows);
            let mut local = Scene::default();
            let mut height = 0.0;
            self.node_uncached(&node, 0.0, &mut height, width, theme, &mut local);
            let cached = Arc::new(Cached {
                width,
                height,
                ops: Arc::new(local.ops),
                hits: std::mem::replace(&mut self.hits, outer_hits),
                rows: std::mem::replace(&mut self.rows, outer_rows),
            });
            self.cache.insert(id.to_string(), cached.clone());
            cached
        };
        scene.ops.push(Op::Group {
            x,
            y: *y,
            ops: cached.ops.clone(),
        });
        let base = self.rows.len();
        self.rows.extend(cached.rows.iter().map(|row| TextRow {
            x: row.x + x,
            y: row.y + *y,
            text: row.text.clone(),
        }));
        self.hits.extend(cached.hits.iter().map(|hit| {
            let mut hit = hit.clone();
            hit.x += x;
            hit.y += *y;
            if let Control::Text(row) = &mut hit.control {
                *row += base;
            }
            hit
        }));
        *y += cached.height;
    }
    pub fn field_text(&self, node: &str, field: &str) -> Option<&str> {
        self.drafts
            .get(&(node.into(), field.into()))
            .map(Editor::text)
    }
    pub fn scroll(&mut self, delta: f32) {
        if let Some(report) = &mut self.report {
            report.offset = report
                .offset
                .saturating_add_signed((delta / 24.0).round() as isize);
            return;
        }
        self.scroll =
            (self.scroll + delta).clamp(0.0, (self.content_height - self.viewport_height).max(0.0));
        self.follow = false;
    }
    pub fn pointer(&mut self, x: f32, y: f32, dragging: bool) -> Vec<Command> {
        if self.picker.is_some() || self.report.is_some() {
            return vec![];
        }
        self.invalidate_focus();
        let commands = self.pointer_inner(x, y, dragging);
        self.invalidate_focus();
        commands
    }
    fn pointer_inner(&mut self, x: f32, y: f32, dragging: bool) -> Vec<Command> {
        let hit = self
            .hits
            .iter()
            .rev()
            .find(|hit| hit.contains(x, y))
            .cloned();
        let Some(hit) = hit else {
            return vec![];
        };
        if let Control::Text(row) = hit.control {
            let cells = ((x - hit.x).max(0.0) / Layout::default().advance).floor() as usize;
            let mut width = 0;
            let column = self
                .rows
                .get(row)
                .map(|row| {
                    row.text
                        .chars()
                        .take_while(|ch| {
                            width += misa_render::width(&ch.to_string());
                            width <= cells
                        })
                        .count()
                })
                .unwrap_or(0);
            if dragging {
                if let Some((_, head)) = &mut self.selection {
                    *head = (row, column);
                }
            } else {
                self.selection = Some(((row, column), (row, column)));
                self.focus = None;
            }
            return vec![];
        }
        if dragging {
            return vec![];
        }
        self.selection = None;
        self.follow = false;
        self.focus = Some(hit.control.clone());
        self.activate(hit.control)
    }
    fn activate(&mut self, control: Control) -> Vec<Command> {
        if let Control::LoadImage(reference) = control {
            return vec![Command::LoadImage(reference)];
        }
        match &control {
            Control::Disclosure(id)
            | Control::Field { node: id, .. }
            | Control::Action { node: id, .. } => self.invalidate(id),
            _ => {}
        }
        match control {
            Control::Disclosure(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Control::Field { node, field } => {
                let kind = self.tree.node(&node).and_then(|node| match &node.kind {
                    Kind::Fields { fields } => fields
                        .iter()
                        .find(|value| value.id == field)
                        .map(|field| field.kind.clone()),
                    _ => None,
                });
                if let Some(edit) = self.drafts.get_mut(&(node, field)) {
                    match kind {
                        Some(FieldKind::Bool) => edit.set_text(if edit.text() == "true" {
                            "false"
                        } else {
                            "true"
                        }),
                        Some(FieldKind::Choice { options, .. }) if !options.is_empty() => {
                            let next = options
                                .iter()
                                .position(|option| option.value == edit.text())
                                .map(|index| (index + 1) % options.len())
                                .unwrap_or(0);
                            edit.set_text(&options[next].value);
                        }
                        _ => {}
                    }
                }
            }
            Control::Action { node, action } if action == "attachment.save" => {
                self.save = Some((node, Editor::new()));
                self.focus = Some(Control::SavePath);
            }
            Control::Action { node, action } => return self.submit(&node, &action),
            Control::SaveConfirm => {
                if let Some((node, path)) = &self.save {
                    if path.text().trim().is_empty() {
                        self.notice = "Enter a local destination path".into();
                    } else {
                        let command = Command::Save {
                            node: node.clone(),
                            destination: path.text().to_string(),
                        };
                        self.save = None;
                        self.focus = None;
                        return vec![command];
                    }
                }
            }
            Control::SaveCancel => {
                self.save = None;
                self.focus = None;
            }
            _ => {}
        }
        vec![]
    }
    fn submit(&mut self, node_id: &str, action_id: &str) -> Vec<Command> {
        let Some(node) = self.tree.node(node_id) else {
            return vec![];
        };
        let Some(action) = node.actions.iter().find(|action| action.id == action_id) else {
            return vec![];
        };
        let mut fields = match &node.kind {
            Kind::Fields { fields } => fields.clone(),
            _ => vec![],
        };
        for field in &mut fields {
            if let Some(edit) = self.drafts.get(&(node_id.into(), field.id.clone())) {
                field.value = edit.text().into();
            }
        }
        let intent = if action_id == "composer.submit" {
            let text = fields
                .iter()
                .find(|field| field.id == "prompt")
                .map(|field| field.value.as_str())
                .unwrap_or("");
            let commands = self.commands.as_slice();
            let parsed = misa_kit::intent::parse(text, commands);
            let Some(intent) = misa_kit::intent::intent(&parsed) else {
                self.notice = format!("Cannot submit: {parsed:?}");
                return vec![];
            };
            intent
        } else {
            Intent::Action {
                node: node_id.into(),
                action: action_id.into(),
                args: action.args.clone(),
                fields,
            }
        };
        // Only the composer hands its draft to a pending operation immediately.
        // Other forms remain locally editable until their owner acknowledges or
        // removes the request; validation/rejection must not erase the input.
        if action_id == "composer.submit" && action.on == ActionOn::Submit {
            for ((node, _), edit) in &mut self.drafts {
                if node == node_id {
                    edit.submit();
                }
            }
        }
        vec![Command::Intent(intent)]
    }
    fn editor(&mut self) -> Option<&mut Editor> {
        match &self.focus {
            Some(Control::SavePath) => self.save.as_mut().map(|(_, edit)| edit),
            Some(Control::Field { node, field }) => {
                self.drafts.get_mut(&(node.clone(), field.clone()))
            }
            _ => None,
        }
    }
    pub fn key(&mut self, key: Key) -> Vec<Command> {
        self.invalidate_focus();
        let commands = self.key_inner(key);
        self.invalidate_focus();
        commands
    }
    fn key_inner(&mut self, key: Key) -> Vec<Command> {
        if let Some(report) = &mut self.report {
            match key {
                Key::Escape | Key::Enter { .. } => self.report = None,
                Key::Up => report.offset = report.offset.saturating_sub(1),
                Key::Down => report.offset = report.offset.saturating_add(1),
                Key::Home => report.offset = 0,
                Key::Copy => return vec![Command::Copy(report.entries.join("\n"))],
                _ => {}
            }
            return vec![];
        }
        if matches!(key, Key::Commands) && self.save.is_none() {
            let mut picker =
                misa_kit::picker::Picker::new("Commands", misa_kit::picker::Accept::Run);
            picker.set_items(
                self.commands.iter().map(|command| misa_proto::view::Choice { value: command.id.clone(), label: command.label.clone(), detail: Some(command.description.clone()) }).collect(),
                false,
            );
            self.picker = Some(picker);
            return vec![];
        }
        if let Some(picker) = &mut self.picker {
            match key {
                Key::Escape => {
                    self.picker = None;
                }
                Key::Up | Key::Tab { backward: true } => picker.move_selection(-1),
                Key::Down | Key::Tab { backward: false } => picker.move_selection(1),
                Key::Text(value) => {
                    for character in value.chars() {
                        picker.type_char(character);
                    }
                }
                Key::Backspace => {
                    picker.backspace();
                }
                Key::Enter { .. } => {
                    if let Some(candidate) = picker.selected().cloned() {
                        let target = self
                            .drafts
                            .keys()
                            .find(|(_, field)| field == "prompt")
                            .cloned();
                        if let Some((node, field)) = target {
                            self.drafts
                                .get_mut(&(node.clone(), field.clone()))
                                .unwrap()
                                .set_text(&format!("/{} ", candidate.value));
                            self.focus = Some(Control::Field { node, field });
                            self.picker = None;
                            self.notice =
                                "Command inserted · add arguments, then Enter to send".into();
                        } else {
                            self.notice = "This view has no prompt field".into();
                        }
                    }
                }
                _ => {}
            }
            return vec![];
        }
        if matches!(key, Key::Copy) {
            let text = if let Some(edit) = self.editor() {
                edit.text().to_string()
            } else {
                self.selected_text()
            };
            return if text.is_empty() {
                vec![]
            } else {
                vec![Command::Copy(text)]
            };
        }
        if matches!(key, Key::SelectAll) {
            if self.editor().is_some() {
                self.replace_selection = true;
            } else if !self.rows.is_empty() {
                self.selection = Some(((0, 0), (self.rows.len() - 1, usize::MAX)));
            }
            return vec![];
        }
        if let Key::Tab { backward } = key {
            let controls: Vec<_> = self
                .hits
                .iter()
                .filter(|hit| !matches!(hit.control, Control::Text(_)))
                .map(|hit| hit.control.clone())
                .collect();
            if !controls.is_empty() {
                let current = controls
                    .iter()
                    .position(|control| Some(control) == self.focus.as_ref());
                let next = match (current, backward) {
                    (Some(index), true) => (index + controls.len() - 1) % controls.len(),
                    (Some(index), false) => (index + 1) % controls.len(),
                    (None, true) => controls.len() - 1,
                    (None, false) => 0,
                };
                self.focus = Some(controls[next].clone());
            }
            return vec![];
        }
        if matches!(key, Key::Escape) {
            if self.save.take().is_some() {
                self.focus = None;
            } else {
                self.selection = None;
            }
            return vec![];
        }
        if let Key::Enter { newline } = key {
            if self.focus == Some(Control::SavePath) {
                return self.activate(Control::SaveConfirm);
            }
            if let Some(Control::Field { node, field }) = self.focus.clone() {
                if newline {
                    if let Some(edit) = self.editor() {
                        edit.insert("\n");
                    }
                    return vec![];
                }
                let action = self
                    .tree
                    .node(&node)
                    .and_then(|node| {
                        node.actions
                            .iter()
                            .find(|action| action.on == ActionOn::Submit)
                    })
                    .map(|action| action.id.clone());
                if let Some(action) = action {
                    return self.submit(&node, &action);
                }
                return self.activate(Control::Field { node, field });
            }
            if let Some(control) = self.focus.clone() {
                return self.activate(control);
            }
        }
        if let Some(control @ Control::Field { .. }) = self.focus.clone() {
            let discrete = match &control {
                Control::Field {node,field} => self.tree.node(node).is_some_and(|node| matches!(&node.kind, Kind::Fields {fields} if fields.iter().any(|value| &value.id == field && matches!(value.kind,FieldKind::Bool|FieldKind::Choice {..})))),
                _ => false,
            };
            if discrete {
                if matches!(&key,Key::Text(value) if value == " ")
                    || matches!(key, Key::Left | Key::Right)
                {
                    return self.activate(control);
                }
                return vec![];
            }
        }
        let replace = self.replace_selection;
        self.replace_selection = false;
        if let Some(edit) = self.editor() {
            match key {
                Key::Text(text) => {
                    if replace {
                        edit.set_text("");
                    }
                    edit.insert(&text);
                }
                Key::Backspace => {
                    if replace {
                        edit.set_text("");
                    } else {
                        edit.backspace();
                    }
                }
                Key::Delete => {
                    edit.delete();
                }
                Key::Left => {
                    edit.move_cursor(Motion::Left);
                }
                Key::Right => {
                    edit.move_cursor(Motion::Right);
                }
                Key::Home => {
                    edit.move_cursor(Motion::LineStart);
                }
                Key::End => {
                    edit.move_cursor(Motion::LineEnd);
                }
                _ => {}
            }
        }
        vec![]
    }
    pub fn selected_text(&self) -> String {
        let Some((a, b)) = self.selection else {
            return String::new();
        };
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        (start.0..=end.0.min(self.rows.len().saturating_sub(1)))
            .filter_map(|row| {
                self.rows.get(row).map(|text| {
                    let from = if row == start.0 { start.1 } else { 0 };
                    let to = if row == end.0 { end.1 } else { usize::MAX };
                    text.text
                        .chars()
                        .skip(from)
                        .take(to.saturating_sub(from))
                        .collect::<String>()
                })
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn frame(&mut self, width: u32, height: u32) -> Scene {
        if self.cache_width != width {
            self.cache.clear();
            self.cache_width = width;
        }
        #[cfg(test)]
        {
            self.rendered_nodes = 0;
        }
        self.viewport_height = height as f32;
        let mut scene = self.layout(width, height);
        let max = (self.content_height - height as f32 + 40.0).max(0.0);
        let wanted = if self.follow {
            max
        } else {
            self.scroll.min(max)
        };
        if (wanted - self.scroll).abs() > 0.5 {
            self.scroll = wanted;
            scene = self.layout(width, height);
        }
        let colors = self.colors();
        if let Some(report) = &mut self.report {
            self.hits.clear();
            let columns = ((width.saturating_sub(96)) as usize / 9).max(1);
            if report.columns != columns {
                report.lines.clear();
                for entry in &report.entries {
                    for line in entry.lines() {
                        let chars: Vec<_> = line.chars().collect();
                        if chars.is_empty() {
                            report.lines.push(String::new());
                        }
                        for chunk in chars.chunks(columns) {
                            report.lines.push(chunk.iter().collect::<String>());
                        }
                    }
                }
                report.columns = columns;
            }
            let visible = (height.saturating_sub(140) / 24).max(1) as usize;
            report.offset = report
                .offset
                .min(report.lines.len().saturating_sub(visible));
            scene.ops.push(Op::Rect {
                x: 24.0,
                y: 24.0,
                width: (width as f32 - 48.0).max(1.0),
                height: (height as f32 - 48.0).max(1.0),
                style: colors.surface,
            });
            scene.ops.push(text(42.0, 40.0, &report.title, colors.text));
            for (index, line) in report
                .lines
                .iter()
                .skip(report.offset)
                .take(visible)
                .enumerate()
            {
                scene
                    .ops
                    .push(text(42.0, 78.0 + index as f32 * 24.0, line, colors.text));
            }
            scene.ops.push(text(
                42.0,
                (height as f32 - 52.0).max(0.0),
                "↑↓ scroll · Escape close",
                colors.muted,
            ));
        }
        scene
    }
    fn layout(&mut self, width: u32, height: u32) -> Scene {
        self.hits.clear();
        self.rows.clear();
        let mut scene = Scene {
            width: width as f32,
            height: height as f32,
            ops: vec![],
        };
        let root = self.root.clone();
        let theme = if self.light {
            Theme::light()
        } else {
            Theme::dark()
        };
        let mut y = 20.0 - self.scroll;
        self.present(
            &root,
            20.0,
            &mut y,
            (width as f32 - 40.0).max(40.0),
            &theme,
            &mut scene,
        );
        self.content_height = y + self.scroll + 20.0;
        if let Some((a, b)) = self.selection {
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            for (index, row) in self
                .rows
                .iter()
                .enumerate()
                .skip(start.0)
                .take(end.0.saturating_sub(start.0) + 1)
            {
                let from = if index == start.0 { start.1 } else { 0 };
                let to = if index == end.0 { end.1 } else { usize::MAX };
                let prefix = row.text.chars().take(from).collect::<String>();
                let selected = row
                    .text
                    .chars()
                    .skip(from)
                    .take(to.saturating_sub(from))
                    .collect::<String>();
                let x = row.x + misa_render::width(&prefix) as f32 * 8.4;
                scene.ops.push(Op::Rect {
                    x,
                    y: row.y,
                    width: misa_render::width(&selected) as f32 * 8.4,
                    height: 21.0,
                    style: self.colors().selection,
                });
                scene
                    .ops
                    .push(text(x, row.y, &selected, self.colors().text));
            }
        }
        if !self.notice.is_empty() {
            scene.ops.push(Op::Rect {
                x: 0.0,
                y: height as f32 - 26.0,
                width: width as f32,
                height: 26.0,
                style: self.colors().surface,
            });
            scene.ops.push(text(
                12.0,
                height as f32 - 23.0,
                &self.notice,
                self.colors().text,
            ));
        }
        if let Some((_, edit)) = &self.save {
            let path = edit.text().to_string();
            let x = 30.0;
            let y = (height as f32 / 2.0 - 70.0).max(20.0);
            let w = (width as f32 - 60.0).max(80.0);
            self.hits.clear();
            scene.ops.push(Op::Rect {
                x,
                y,
                width: w,
                height: 150.0,
                style: self.colors().surface,
            });
            scene.ops.push(text(
                x + 12.0,
                y + 10.0,
                "Save attachment · local destination",
                self.colors().text,
            ));
            self.box_control(
                &mut scene,
                x + 12.0,
                y + 40.0,
                w - 24.0,
                32.0,
                &path,
                Control::SavePath,
            );
            self.box_control(
                &mut scene,
                x + 12.0,
                y + 92.0,
                95.0,
                32.0,
                "Save",
                Control::SaveConfirm,
            );
            self.box_control(
                &mut scene,
                x + 120.0,
                y + 92.0,
                95.0,
                32.0,
                "Cancel",
                Control::SaveCancel,
            );
        }
        if let Some(picker) = &self.picker {
            self.hits.clear();
            let y = 35.0;
            scene.ops.push(Op::Rect {
                x: 30.0,
                y,
                width: (width as f32 - 60.0).max(80.0),
                height: 300.0,
                style: self.colors().surface,
            });
            scene.ops.push(text(
                42.0,
                y + 12.0,
                &format!("Commands · {}", picker.query),
                self.colors().text,
            ));
            let matches = picker.matches();
            let start = picker.selected_index().saturating_sub(7);
            if matches.is_empty() {
                scene.ops.push(text(
                    42.0,
                    y + 48.0,
                    "No matching commands",
                    self.colors().muted,
                ));
            }
            for (index, candidate) in matches.iter().enumerate().skip(start).take(8) {
                let label = format!(
                    "{} /{} · {}",
                    if index == picker.selected_index() {
                        ">"
                    } else {
                        " "
                    },
                    candidate.value,
                    candidate.label
                );
                scene.ops.push(text(
                    42.0,
                    y + 48.0 + (index - start) as f32 * 26.0,
                    &label,
                    self.colors().text,
                ));
            }
            scene.ops.push(text(
                42.0,
                y + 270.0,
                "↑↓ select · Enter insert · Escape close",
                self.colors().muted,
            ));
        }
        scene
    }
    fn box_control(
        &mut self,
        scene: &mut Scene,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        label: &str,
        control: Control,
    ) {
        let focused = self.focus.as_ref() == Some(&control);
        scene.ops.push(Op::Rect {
            x: x - 1.0,
            y: y - 1.0,
            width: width + 2.0,
            height: height + 2.0,
            style: if focused {
                self.colors().accent
            } else {
                self.colors().border
            },
        });
        scene.ops.push(Op::Rect {
            x,
            y,
            width,
            height,
            style: self.colors().field,
        });
        for (line, text_value) in label.lines().enumerate() {
            scene.ops.push(text(
                x + 7.0,
                y + 6.0 + line as f32 * 21.0,
                text_value,
                self.colors().text,
            ));
        }
        if focused && matches!(control, Control::Field { .. } | Control::SavePath) {
            if let Some(edit) = self.editor() {
                let before = &edit.text()[..edit.cursor()];
                let line = before.chars().filter(|ch| *ch == '\n').count();
                let column = before.rsplit('\n').next().unwrap_or("").chars().count();
                scene.ops.push(Op::Rect {
                    x: (x + 7.0 + column as f32 * 8.4).min(x + width - 3.0),
                    y: y + 6.0 + line as f32 * 21.0,
                    width: 1.5,
                    height: 18.0,
                    style: self.colors().text,
                });
            }
        }
        self.hits.push(Hit {
            x,
            y,
            width,
            height,
            control,
        });
    }
    fn row(&mut self, scene: &mut Scene, x: f32, y: f32, spans: Vec<(Style, String)>) {
        let plain = spans
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<String>();
        let index = self.rows.len();
        let mut xx = x;
        for (style, value) in spans {
            scene.ops.push(text(xx, y, &value, style));
            xx += misa_render::width(&value) as f32 * 8.4;
        }
        self.hits.push(Hit {
            x,
            y,
            width: (xx - x).max(8.4),
            height: 21.0,
            control: Control::Text(index),
        });
        self.rows.push(TextRow { x, y, text: plain });
    }
    fn node_uncached(
        &mut self,
        node: &Node,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        if misa_render::components::render_default(node, theme, (width / 8.4).max(1.0) as usize)
            .is_some()
        {
            // Indexed nodes contain no children. A registered composite owns its
            // subtree, so materialize that subtree only when its cache is dirty.
            let model = self.tree.subtree(&node.id).unwrap_or_else(|| node.clone());
            let lines = misa_render::components::render_default(
                &model,
                theme,
                (width / 8.4).max(1.0) as usize,
            )
            .expect("same registered role");
            for line in lines {
                self.row(scene, x, *y, line.spans);
                *y += 21.0;
            }
            return;
        }
        #[cfg(test)]
        {
            self.rendered_nodes += 1;
        }
        if let Some(label) = &node.label {
            self.row(scene, x, *y, vec![(theme.role(&node.role), label.clone())]);
            *y += 25.0;
        }
        let mut children = true;
        match &node.kind {
            Kind::Section => {}
            Kind::Collapsible { summary } => {
                let open = self.expanded.contains(&node.id);
                let label = format!(
                    "{} {}",
                    if open { "▾" } else { "▸" },
                    summary
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>()
                );
                self.box_control(
                    scene,
                    x,
                    *y,
                    width,
                    28.0,
                    &label,
                    Control::Disclosure(node.id.clone()),
                );
                *y += 34.0;
                children = open;
            }
            Kind::Fields { fields } => {
                for field in fields {
                    self.row(
                        scene,
                        x,
                        *y,
                        vec![(theme.role("field.label"), field.label.clone())],
                    );
                    *y += 22.0;
                    let value = self
                        .field_text(&node.id, &field.id)
                        .unwrap_or(&field.value)
                        .to_string();
                    let display = if field.secret {
                        "•".repeat(value.chars().count())
                    } else {
                        match &field.kind {
                            FieldKind::Bool => {
                                format!("[{}]", if value == "true" { "✓" } else { " " })
                            }
                            FieldKind::Choice { options, .. } => format!(
                                "{} ▾",
                                options
                                    .iter()
                                    .find(|choice| choice.value == value)
                                    .map(|choice| choice.label.as_str())
                                    .unwrap_or(&value)
                            ),
                            _ => value.clone(),
                        }
                    };
                    let h = if field.kind == FieldKind::Block {
                        (display.lines().count().max(3) as f32 * 21.0 + 12.0).min(180.0)
                    } else {
                        32.0
                    };
                    if field.read_only {
                        for line in display.lines() {
                            self.row(scene, x, *y, vec![(theme.role("value.text"), line.into())]);
                            *y += 21.0;
                        }
                    } else {
                        self.box_control(
                            scene,
                            x,
                            *y,
                            width,
                            h,
                            &display,
                            Control::Field {
                                node: node.id.clone(),
                                field: field.id.clone(),
                            },
                        );
                        *y += h;
                    }
                    *y += 9.0;
                }
            }
            Kind::Meter { label, value, max } => {
                self.row(
                    scene,
                    x,
                    *y,
                    vec![(
                        theme.role(&node.role),
                        format!("{label}: {value:.1} / {max:.1}"),
                    )],
                );
                *y += 24.0;
                scene.ops.push(Op::Rect {
                    x,
                    y: *y,
                    width,
                    height: 10.0,
                    style: self.colors().meter,
                });
                scene.ops.push(Op::Rect {
                    x,
                    y: *y,
                    width: width
                        * if *max > 0.0 {
                            (value / max).clamp(0.0, 1.0) as f32
                        } else {
                            0.0
                        },
                    height: 10.0,
                    style: self.colors().accent,
                });
                *y += 22.0;
            }
            Kind::Table { head, rows } => {
                let columns = head
                    .len()
                    .max(rows.iter().map(Vec::len).max().unwrap_or(1))
                    .max(1);
                let cell_width = width / columns as f32;
                for (index, row) in std::iter::once(head).chain(rows.iter()).enumerate() {
                    let cells: Vec<_> = row
                        .iter()
                        .map(|cell| {
                            let leaf = Node::text("table.cell", cell.clone());
                            misa_render::render(
                                &leaf,
                                theme,
                                ((cell_width - 10.0) / 8.4).max(1.0) as usize,
                            )
                        })
                        .collect();
                    let lines = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
                    let height = lines as f32 * 21.0 + 8.0;
                    scene.ops.push(Op::Rect {
                        x,
                        y: *y,
                        width,
                        height,
                        style: if index == 0 {
                            self.colors().selected_button
                        } else {
                            self.colors().button
                        },
                    });
                    for (column, cell) in cells.into_iter().enumerate() {
                        for (line, content) in cell.into_iter().enumerate() {
                            self.row(
                                scene,
                                x + column as f32 * cell_width + 5.0,
                                *y + 4.0 + line as f32 * 21.0,
                                content.spans,
                            );
                        }
                    }
                    *y += height + 2.0;
                }
            }
            Kind::List { ordered, items } => {
                for (index, item) in items.iter().enumerate() {
                    self.row(
                        scene,
                        x,
                        *y,
                        vec![(
                            theme.role(&node.role),
                            if *ordered {
                                format!("{}.", index + 1)
                            } else {
                                "•".into()
                            },
                        )],
                    );
                    for child in item {
                        self.node_uncached(
                            child,
                            x + 25.0,
                            y,
                            (width - 25.0).max(10.0),
                            theme,
                            scene,
                        );
                    }
                }
            }
            Kind::Image { blob, alt, .. } => {
                if let Some(image) = self.images.get(&blob.hash) {
                    let scale = (width / image.width() as f32)
                        .min(320.0 / image.height() as f32)
                        .min(1.0);
                    let (w, h) = (image.width() as f32 * scale, image.height() as f32 * scale);
                    scene.ops.push(Op::Image {
                        x,
                        y: *y,
                        width: w,
                        height: h,
                        image: image.clone(),
                    });
                    *y += h + 6.0;
                } else {
                    self.box_control(
                        scene,
                        x,
                        *y,
                        width.min(180.0),
                        30.0,
                        "Load image",
                        Control::LoadImage(blob.clone()),
                    );
                    *y += 36.0;
                }
                self.row(scene, x, *y, vec![(theme.role(&node.role), alt.clone())]);
                *y += 25.0;
            }
            _ => {
                let mut leaf = node.clone();
                leaf.children.clear();
                leaf.actions.clear();
                leaf.label = None;
                for line in
                    misa_render::render(&leaf, theme, (width / 8.4).floor().max(1.0) as usize)
                {
                    self.row(scene, x + line.indent as f32 * 8.4, *y, line.spans);
                    *y += 21.0;
                }
            }
        }
        if children {
            for id in self.tree.children(&node.id) {
                self.present(&id, x, y, width, theme, scene);
            }
            for child in &node.children {
                self.node_uncached(child, x, y, width, theme, scene);
            }
            if node.id == self.stream_parent() && !self.visible_streams().is_empty() {
                self.present("streams", x, y, width, theme, scene);
            }
            if node.id == "streams" {
                let ids = self.visible_streams();
                for id in ids {
                    self.present(&id, x, y, width, theme, scene);
                }
            }
        }
        for action in &node.actions {
            self.box_control(
                scene,
                x,
                *y,
                width.min(260.0),
                32.0,
                action.label.as_deref().unwrap_or(&action.id),
                Control::Action {
                    node: node.id.clone(),
                    action: action.id.clone(),
                },
            );
            *y += 39.0;
        }
        *y += 5.0;
    }
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
fn text(x: f32, y: f32, value: &str, style: Style) -> Op {
    Op::Text {
        x,
        y,
        size: 15.0,
        style,
        text: value.into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn changing_local_theme_preserves_drafts_and_rebuilds_cached_colors() {
        let mut app = App::new(form("panel.input", FieldKind::Inline));
        app.frame(640, 480);
        app.key(Key::Text("private draft".into()));
        let dark = app.frame(640, 480);
        app.set_light(true);
        let light = app.frame(640, 480);
        assert_eq!(
            app.field_text("panel.input", "value"),
            Some("private draft")
        );
        assert_ne!(dark.ops, light.ops);
        app.set_light(false);
        assert_eq!(dark.ops, app.frame(640, 480).ops);
    }
    use super::*;
    use misa_proto::view::{Action, Field, Span};
    use misa_value::Value;
    #[test]
    fn command_picker_filters_navigates_and_inserts_without_sending() {
        let mut view = form("compose", FieldKind::Inline);
        if let Kind::Fields { fields } = &mut view.kind {
            fields[0].id = "prompt".into();
        }
        let mut app = App::new(view);
        app.commands = serde_json::from_value(serde_json::json!([{ "id":"model", "label":"Model" }, { "id":"clear", "label":"Clear" }])).unwrap();
        app.key(Key::Commands);
        app.key(Key::Down);
        assert_eq!(
            app.picker.as_ref().unwrap().selected().unwrap().value,
            "clear"
        );
        app.key(Key::Text("mod".into()));
        assert_eq!(app.picker.as_ref().unwrap().matches().len(), 1);
        assert!(app.key(Key::Enter { newline: false }).is_empty());
        assert_eq!(app.field_text("compose", "prompt"), Some("/model "));
        assert!(app.picker.is_none());
        app.key(Key::Commands);
        app.key(Key::Text("zzzz".into()));
        assert!(app.key(Key::Enter { newline: false }).is_empty());
        assert!(app.picker.is_some());
        app.key(Key::Escape);
        assert_eq!(app.field_text("compose", "prompt"), Some("/model "));
    }
    #[test]
    fn empty_picker_and_modal_input_preserve_draft() {
        let mut app = App::new(form("form", FieldKind::Inline));
        app.frame(900, 720);
        app.key(Key::Commands);
        app.key(Key::Text("query".into()));
        assert!(app.pointer(80.0, 55.0, false).is_empty());
        assert!(app.key(Key::Enter { newline: false }).is_empty());
        let scene = app.frame(900, 720);
        assert!(any_op(
            &scene.ops,
            |op| matches!(op,Op::Text{text,..} if text == "No matching commands")
        ));
        app.key(Key::Escape);
        assert!(app.picker.is_none());
    }
    fn any_op(ops: &[Op], predicate: fn(&Op) -> bool) -> bool {
        ops.iter()
            .any(|op| predicate(op) || matches!(op,Op::Group {ops,..} if any_op(ops,predicate)))
    }
    #[test]
    fn registered_composite_receives_its_indexed_subtree() {
        let view = Node::section("status.indicators").id("indicators").child(
            Node::section("indicator.model").id("model").child(
                Node::new(
                    "value.text",
                    Kind::Fact {
                        value: Value::str("scripted/model"),
                    },
                )
                .id("model.value"),
            ),
        );
        let mut app = App::new(view);
        let scene = app.frame(900, 120);
        assert!(any_op(
            &scene.ops,
            |op| matches!(op, Op::Text { text, .. } if text.contains("scripted/model"))
        ));
        app.apply_tree([&ViewOp::Replace {
            id: "model.value".into(),
            node: Node::new(
                "value.text",
                Kind::Fact {
                    value: Value::str("changed/model"),
                },
            )
            .id("model.value"),
        }])
        .unwrap();
        let scene = app.frame(900, 120);
        assert!(any_op(
            &scene.ops,
            |op| matches!(op, Op::Text { text, .. } if text.contains("changed/model"))
        ));
    }
    fn form(id: &str, kind: FieldKind) -> Node {
        Node::new(
            "panel",
            Kind::Fields {
                fields: vec![Field {
                    id: "value".into(),
                    label: "Value".into(),
                    value: String::new(),
                    hint: None,
                    kind,
                    read_only: false,
                    secret: false,
                }],
            },
        )
        .id(id)
        .action(Action {
            id: "answer".into(),
            on: ActionOn::Submit,
            label: None,
            args: Value::Null,
        })
    }
    #[test]
    fn reports_and_rejected_prompts_preserve_local_typing() {
        let mut view = form("composer", FieldKind::Inline);
        let Kind::Fields { fields } = &mut view.kind else {
            unreachable!()
        };
        fields[0].id = "prompt".into();
        let mut app = App::new(view.clone());
        app.focus = Some(Control::Field {
            node: "composer".into(),
            field: "prompt".into(),
        });
        app.key(Key::Text("new draft".into()));
        app.report(
            "Status".into(),
            Value::map([("needs_input", Value::Bool(true))]),
        );
        assert!(
            app.key(Key::Text("must not reach composer".into()))
                .is_empty()
        );
        let scene = app.frame(800, 600);
        assert!(any_op(
            &scene.ops,
            |op| matches!(op, Op::Text {text, ..} if text == "needs input: yes")
        ));
        assert!(app.key(Key::Escape).is_empty());
        assert_eq!(app.field_text("composer", "prompt"), Some("new draft"));
        app.reject_prompt(
            "previous submission".into(),
            "Reply lost; execution is uncertain".into(),
        );
        assert_eq!(app.field_text("composer", "prompt"), Some("new draft"));
        assert_eq!(
            app.key(Key::Copy),
            vec![Command::Copy("previous submission".into())]
        );
        app.key(Key::Escape);
        app.drafts
            .get_mut(&("composer".into(), "prompt".into()))
            .unwrap()
            .set_text("");
        app.reject_prompt("restored".into(), "Rejected".into());
        assert_eq!(app.field_text("composer", "prompt"), Some("restored"));
        assert_eq!(
            app.tree.snapshot(),
            view,
            "local reports must not change authoritative content"
        );
    }
    #[test]
    fn drafts_survive_updates_and_submit_only_the_target_panel() {
        let view = Node::section("root")
            .child(form("one", FieldKind::Inline))
            .child(form("two", FieldKind::Inline));
        let mut app = App::new(view.clone());
        app.focus = Some(Control::Field {
            node: "one".into(),
            field: "value".into(),
        });
        app.key(Key::Text("private draft".into()));
        app.set_view(view);
        assert_eq!(app.field_text("one", "value"), Some("private draft"));
        let sent = app.submit("two", "answer");
        assert!(
            matches!(&sent[..],[Command::Intent(Intent::Action {node,fields,..})] if node=="two" && fields[0].value.is_empty())
        );
        assert_eq!(app.field_text("one", "value"), Some("private draft"));
    }
    #[test]
    fn boolean_keyboard_input_cannot_produce_invalid_values() {
        let mut app = App::new(form("one", FieldKind::Bool));
        app.key(Key::Text("nonsense".into()));
        assert_eq!(app.field_text("one", "value"), Some(""));
        app.key(Key::Text(" ".into()));
        assert_eq!(app.field_text("one", "value"), Some("true"));
        app.key(Key::Text(" ".into()));
        assert_eq!(app.field_text("one", "value"), Some("false"));
    }
    #[test]
    fn disclosure_state_and_unicode_copy_are_local() {
        let view = Node::new(
            "tool",
            Kind::Collapsible {
                summary: vec![Span::plain("Details")],
            },
        )
        .id("tool")
        .child(Node::text("text", [Span::plain("héllo λ")]));
        let mut app = App::new(view.clone());
        app.activate(Control::Disclosure("tool".into()));
        app.set_view(view);
        app.frame(500, 500);
        app.focus = None;
        app.key(Key::SelectAll);
        assert_eq!(app.key(Key::Copy), vec![Command::Copy("héllo λ".into())]);
        assert!(app.expanded.contains("tool"));
    }
    #[test]
    fn save_destination_is_an_explicit_local_command() {
        let mut app = App::new(Node::section("root"));
        app.activate(Control::Action {
            node: "attachment".into(),
            action: "attachment.save".into(),
        });
        app.key(Key::Text("/tmp/my photo.png".into()));
        assert_eq!(
            app.key(Key::Enter { newline: false }),
            vec![Command::Save {
                node: "attachment".into(),
                destination: "/tmp/my photo.png".into()
            }]
        );
        assert!(app.save.is_none());
    }
    #[test]
    fn rich_shapes_draw_wrapped_table_meter_and_bitmap() {
        let view = Node::section("root")
            .child(Node::new(
                "table",
                Kind::Table {
                    head: vec![vec![Span::plain("Heading")]],
                    rows: vec![vec![vec![Span::plain(
                        "A long cell that wraps into multiple visible lines",
                    )]]],
                },
            ))
            .child(Node::new(
                "meter",
                Kind::Meter {
                    label: "Used".into(),
                    value: 5.0,
                    max: 10.0,
                },
            ))
            .child(Node::new(
                "image",
                Kind::Image {
                    blob: misa_proto::view::BlobRef {
                        hash: "image".into(),
                        len: 4,
                        media: Some("image/png".into()),
                    },
                    alt: "Picture".into(),
                    width: 1,
                    height: 1,
                },
            ));
        let mut app = App::new(view);
        app.images.insert(
            "image".into(),
            Arc::new(image::RgbaImage::from_pixel(
                1,
                1,
                image::Rgba([255, 0, 0, 255]),
            )),
        );
        let scene = app.frame(200, 600);
        assert!(any_op(&scene.ops, |op| matches!(op, Op::Image { .. })));
        assert!(any_op(
            &scene.ops,
            |op| matches!(op,Op::Rect {width,height,..} if *width==80.0 && *height==10.0)
        ));
        assert!(app.rows.len() > 6, "long table cell must wrap");
        let raster = crate::paint::raster(&scene, Color::Rgb(20, 22, 26)).unwrap();
        assert!(raster.pixels().any(|pixel| pixel.0 == [255, 0, 0, 255]));
    }

    #[test]
    fn evicted_images_release_retained_scenes_and_can_be_reloaded() {
        let reference = |hash: &str| misa_proto::view::BlobRef {
            hash: hash.into(),
            len: 4,
            media: Some("image/png".into()),
        };
        let node = |hash: &str| {
            Node::new(
                "image",
                Kind::Image {
                    blob: reference(hash),
                    alt: hash.into(),
                    width: 2048,
                    height: 2048,
                },
            )
            .id(hash)
        };
        let mut app = App::new(
            Node::section("root")
                .id("root")
                .child(node("a"))
                .child(node("b"))
                .child(node("c")),
        );
        let first = Arc::new(image::RgbaImage::new(2048, 2048));
        let weak = Arc::downgrade(&first);
        app.image("a".into(), first);
        drop(app.frame(800, 600));
        app.image("b".into(), Arc::new(image::RgbaImage::new(2048, 2048)));
        app.image("c".into(), Arc::new(image::RgbaImage::new(2048, 2048)));
        assert!(
            weak.upgrade().is_none(),
            "Cached display lists must not pin evicted bytes"
        );
        assert!(
            app.images
                .values()
                .map(|image| image.as_raw().len())
                .sum::<usize>()
                <= 32 * 1024 * 1024
        );
        app.frame(800, 600);
        assert!(
            app.hits
                .iter()
                .any(|hit| hit.control == Control::LoadImage(reference("a")))
        );
        assert_eq!(
            app.activate(Control::LoadImage(reference("a"))),
            vec![Command::LoadImage(reference("a"))]
        );
        app.image("a".into(), Arc::new(image::RgbaImage::new(1, 1)));
        app.apply_tree([&ViewOp::Remove { id: "a".into() }])
            .unwrap();
        assert!(!app.images.contains_key("a"));
        app.image("a".into(), Arc::new(image::RgbaImage::new(1, 1)));
        assert!(
            !app.images.contains_key("a"),
            "Late decode cannot repopulate a removed owner"
        );
    }
    #[test]
    fn live_updates_do_not_steal_the_local_save_dialog() {
        let view = form("panel.input", FieldKind::Inline);
        let mut app = App::new(view.clone());
        app.activate(Control::Action {
            node: "image".into(),
            action: "attachment.save".into(),
        });
        app.key(Key::Text("/tmp/".into()));
        app.set_view(view);
        app.key(Key::Text("photo.png".into()));
        assert_eq!(
            app.key(Key::Enter { newline: false }),
            vec![Command::Save {
                node: "image".into(),
                destination: "/tmp/photo.png".into()
            }]
        );
    }
    #[test]
    fn pointer_selection_counts_wide_characters_as_display_cells() {
        let mut app = App::new(Node::text("text", [Span::plain("界hi")]));
        app.frame(400, 200);
        app.pointer(20.0 + 2.1 * 8.4, 25.0, false);
        app.pointer(20.0 + 3.1 * 8.4, 25.0, true);
        assert_eq!(app.selected_text(), "h");
    }
    #[test]
    fn deterministic_raster_oracle_and_unchanged_frames_do_no_layout() {
        for owners in [10, 1000] {
            let view = Node::section("session").id("session").child(
                Node::section("transcript")
                    .id("transcript")
                    .children((0..owners).map(|index| {
                        Node::text("message", [Span::plain("unchanged transcript")])
                            .id(format!("message.{index}"))
                    })),
            );
            let mut app = App::new(view.clone());
            let mut hashes = std::collections::BTreeSet::new();
            for _ in 0..6 {
                app.set_view(view.clone());
                let scene = app.frame(800, 600);
                let bytes = crate::paint::png(&scene, Color::Rgb(20, 22, 26)).unwrap();
                hashes.insert(bytes);
            }
            assert_eq!(hashes.len(), 1, "render oracle is not deterministic");
            app.frame(800, 600);
            assert_eq!(app.rendered_nodes, 0);
        }
    }
    #[test]
    fn scoped_document_transaction_settles_live_text_without_rebuilding_history() {
        use misa_client::document::Update;
        use misa_proto::{
            observation::Document,
            sync::{Stream, Version},
        };
        use misa_protocol::observation::{Applied, MemberChange};
        for owners in [10, 1000] {
            let misa_proto::SessionMsg::View { view, .. } = protocol_view(owners) else {
                unreachable!()
            };
            let mut app = App::new(Node::section("empty"));
            app.observed(&Update::Reset(Document {
                version: Version {
                    epoch: "test".into(),
                    rev: 0,
                },
                tree: view.clone(),
                streams: vec![Stream {
                    id: "answer.text".into(),
                    role: "message.assistant".into(),
                    text: "é".into(),
                }],
            }))
            .unwrap();
            app.frame(800, 600);
            let retained = app.cache["message.0"].ops.clone();
            let answer = Node::text("message.assistant", [Span::plain("é終")]).id("answer");
            let update = Update::Changed {
                member: "body".into(),
                applied: Arc::new(Applied::Changed(std::collections::BTreeMap::from([(
                    "body".into(),
                    MemberChange::Document {
                        tree: vec![ViewOp::Insert {
                            parent: "transcript".into(),
                            before: None,
                            node: answer.clone(),
                        }],
                        live: vec![StreamUpdate::End {
                            id: "answer.text".into(),
                        }],
                        reset_live: false,
                    },
                )]))),
            };
            app.observed(&update).unwrap();
            assert!(app.streams.is_empty());
            assert!(app.tree.contains("answer"));
            let scene = app.frame(800, 600);
            assert!(Arc::ptr_eq(&retained, &app.cache["message.0"].ops));
            assert!(
                app.rendered_nodes <= 3,
                "only changed ancestors and final answer need layout"
            );
            let mut expected = view;
            expected.children[0].children.push(answer);
            let mut cold = App::new(expected);
            assert_eq!(
                crate::paint::raster(&scene, Color::Rgb(20, 22, 26)).unwrap(),
                crate::paint::raster(&cold.frame(800, 600), Color::Rgb(20, 22, 26)).unwrap()
            );
        }
    }
    fn protocol_view(owners: usize) -> misa_proto::SessionMsg {
        misa_proto::SessionMsg::View {
            id: misa_proto::SubId(1),
            version: misa_proto::sync::Version {
                epoch: "test".into(),
                rev: 0,
            },
            view: Node::section("session").id("session").child(
                Node::section("transcript")
                    .id("transcript")
                    .children((0..owners).map(|index| {
                        Node::text("message", [Span::plain("unchanged transcript")])
                            .id(format!("message.{index}"))
                    })),
            ),
        }
    }
    fn apply(
        app: &mut App,
        oracle: &mut misa_proto::sync::ClientView,
        message: misa_proto::SessionMsg,
    ) {
        oracle.receive(&message).unwrap();
        app.receive(&message).unwrap();
    }
    fn assert_cold_pixels(app: &mut App, oracle: &misa_proto::sync::ClientView) {
        let scene = app.frame(800, 600);
        let mut cold = App::new(oracle.rendered().unwrap());
        let expected = cold.frame(800, 600);
        assert_eq!(
            crate::paint::raster(&scene, Color::Rgb(20, 22, 26)).unwrap(),
            crate::paint::raster(&expected, Color::Rgb(20, 22, 26)).unwrap()
        );
    }
    #[test]
    fn stream_append_and_subtree_replace_reuse_unchanged_owner_scenes() {
        use misa_proto::{
            SessionEvent, SessionMsg,
            sync::{Change, ClientView, Stream, StreamUpdate, Version},
        };
        for owners in [10, 1000] {
            let mut app = App::new(Node::section("empty"));
            let mut oracle = ClientView::default();
            apply(&mut app, &mut oracle, protocol_view(owners));
            app.frame(800, 600);
            let retained: Vec<_> = (0..owners)
                .map(|index| app.cache[&format!("message.{index}")].ops.clone())
                .collect();
            apply(
                &mut app,
                &mut oracle,
                SessionMsg::Streams {
                    streams: vec![Stream {
                        id: "live.text".into(),
                        role: "message.assistant".into(),
                        text: "hello".into(),
                    }],
                },
            );
            app.frame(800, 600);
            apply(
                &mut app,
                &mut oracle,
                SessionMsg::Event {
                    seq: 1,
                    event: SessionEvent::Stream {
                        update: StreamUpdate::Append {
                            id: "live.text".into(),
                            offset: 5,
                            text: " world".into(),
                        },
                    },
                },
            );
            app.frame(800, 600);
            assert_eq!(
                app.rendered_nodes, 4,
                "only root, transcript, stream group and changed stream lay out"
            );
            for (index, ops) in retained.iter().enumerate() {
                assert!(Arc::ptr_eq(
                    ops,
                    &app.cache[&format!("message.{index}")].ops
                ));
            }
            assert_cold_pixels(&mut app, &oracle);
            apply(
                &mut app,
                &mut oracle,
                SessionMsg::Changes {
                    id: misa_proto::SubId(1),
                    changes: vec![Change {
                        from: Version {
                            epoch: "test".into(),
                            rev: 0,
                        },
                        version: Version {
                            epoch: "test".into(),
                            rev: 1,
                        },
                        ops: vec![ViewOp::Replace {
                            id: "message.0".into(),
                            node: Node::text("message", [Span::plain("changed")]).id("message.0"),
                        }],
                    }],
                },
            );
            app.frame(800, 600);
            assert_eq!(
                app.rendered_nodes, 3,
                "only root, transcript and replaced owner lay out"
            );
            for (index, ops) in retained.iter().enumerate().skip(1) {
                assert!(Arc::ptr_eq(
                    ops,
                    &app.cache[&format!("message.{index}")].ops
                ));
            }
            assert_cold_pixels(&mut app, &oracle);
        }
    }
    #[test]
    fn stream_completion_and_owner_removal_match_cold_rebuilds() {
        use misa_proto::{
            SessionEvent, SessionMsg,
            sync::{Change, ClientView, Stream, StreamUpdate, Version},
        };
        let mut app = App::new(Node::section("empty"));
        let mut oracle = ClientView::default();
        apply(&mut app, &mut oracle, protocol_view(4));
        apply(
            &mut app,
            &mut oracle,
            SessionMsg::Streams {
                streams: vec![Stream {
                    id: "live.text".into(),
                    role: "message.assistant".into(),
                    text: "streamed answer".into(),
                }],
            },
        );
        assert_cold_pixels(&mut app, &oracle);
        apply(
            &mut app,
            &mut oracle,
            SessionMsg::Changes {
                id: misa_proto::SubId(1),
                changes: vec![Change {
                    from: Version {
                        epoch: "test".into(),
                        rev: 0,
                    },
                    version: Version {
                        epoch: "test".into(),
                        rev: 1,
                    },
                    ops: vec![
                        ViewOp::Insert {
                            parent: "transcript".into(),
                            before: Some("message.2".into()),
                            node: Node::text("message", [Span::plain("committed answer")])
                                .id("live"),
                        },
                        ViewOp::Remove {
                            id: "message.0".into(),
                        },
                    ],
                }],
            },
        );
        assert_cold_pixels(&mut app, &oracle);
        apply(
            &mut app,
            &mut oracle,
            SessionMsg::Event {
                seq: 1,
                event: SessionEvent::Stream {
                    update: StreamUpdate::End {
                        id: "live.text".into(),
                    },
                },
            },
        );
        assert_cold_pixels(&mut app, &oracle);
    }
    #[test]
    fn editing_a_field_does_not_relayout_the_transcript() {
        let view = Node::section("session")
            .id("session")
            .child(
                Node::section("transcript")
                    .id("transcript")
                    .child(Node::text("message", [Span::plain("old text")]).id("message.1")),
            )
            .child(form("panel.input", FieldKind::Inline));
        let mut app = App::new(view);
        app.frame(800, 600);
        let owner = app.cache["transcript"].ops.clone();
        app.key(Key::Text("draft".into()));
        app.frame(800, 600);
        assert_eq!(app.rendered_nodes, 2);
        assert!(Arc::ptr_eq(&owner, &app.cache["transcript"].ops));
        assert_eq!(app.field_text("panel.input", "value"), Some("draft"));
        let sent = app.key(Key::Enter { newline: false });
        assert!(
            matches!(&sent[..],[Command::Intent(Intent::Action {fields,..})] if fields[0].value=="draft")
        );
        assert_eq!(app.field_text("panel.input", "value"), Some("draft"));
    }
}
