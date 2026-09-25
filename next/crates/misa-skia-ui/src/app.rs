//! Client-owned interaction and layout. No field draft, disclosure state or destination leaves
//! this module until the person activates an action the session advertised.
use crate::{Op, Scene, TextMetrics};
use misa_kit::editor::{Editor, Motion};
use misa_kit::intent::Intent;
use misa_proto::sync::{IndexedTree, StreamUpdate, ViewOp};
use misa_proto::view::{ActionOn, FieldKind, Kind, Node};
use misa_render::Style;
#[cfg(test)]
use misa_render::Theme;
use misa_value::Value;
pub use misa_window_core::Key;
use misa_window_core::{Event, Output, Size};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

mod layout;
#[cfg(test)]
mod tests;
mod text;

/// Pulse cadence shared by the fake clock and the window scheduler.
pub const PULSE_PERIOD: Duration = Duration::from_millis(160);

fn pulse_phase(elapsed: Duration) -> u64 {
    // The stock pulse has four frames; reduce before converting an arbitrary Duration.
    ((elapsed.as_nanos() / PULSE_PERIOD.as_nanos()) % 4) as u64
}

/// A validated document transaction, independent of its delivery mechanism.
/// Tree edits and live-stream retirement are applied together before painting.
#[derive(Clone, Copy, Debug)]
pub enum DocumentUpdate<'a> {
    Reset {
        tree: &'a Node,
        streams: &'a [misa_proto::sync::Stream],
    },
    Changed {
        tree: &'a [ViewOp],
        live: &'a [StreamUpdate],
        reset_live: bool,
    },
    Notice(&'a str),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Intent(Intent),
    Save { node: String, destination: String },
    LoadImage(misa_proto::view::BlobRef),
    Copy(String),
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
#[derive(Debug)]
struct RowGeometry {
    text: String,
    /// Measured caret positions, one per Unicode scalar boundary.
    advances: Vec<f32>,
    runs: Vec<(Style, String, usize)>,
}
#[derive(Clone, Debug)]
struct TextRow {
    x: f32,
    y: f32,
    /// The same local viewport used by the paint op and the hit rectangle.
    width: f32,
    geometry: Arc<RowGeometry>,
}
impl TextRow {
    fn column(&self, x: f32) -> usize {
        self.geometry
            .advances
            .partition_point(|edge| *edge <= x - self.x)
            .saturating_sub(1)
    }
    fn edge(&self, column: usize) -> f32 {
        let advances = &self.geometry.advances;
        advances[column.min(advances.len() - 1)].min(self.width)
    }
}
#[derive(Clone, Copy, Default)]
struct FieldViewport {
    x: f32,
    line: usize,
}
#[derive(Clone)]
struct IndicatorBounds {
    id: String,
    top: f32,
    bottom: f32,
}
#[derive(Clone)]
struct Cached {
    width: f32,
    height: f32,
    ops: Arc<Vec<Op>>,
    hits: Vec<Hit>,
    rows: Vec<TextRow>,
    /// Moving owner groups, relative to this cached group's origin.
    indicators: Vec<IndicatorBounds>,
    /// Pulse phase when this moving status owner's display list was built.
    phase: Option<u64>,
}
pub struct App {
    light: bool,
    metrics: Arc<dyn TextMetrics>,
    #[cfg(test)]
    rendered_nodes: usize,
    tree: IndexedTree,
    root: String,
    streams: BTreeMap<String, Node>,
    cache: BTreeMap<String, Arc<Cached>>,
    /// Painted status owners with a moving activity indicator (not document-wide turns).
    moving_indicators: BTreeSet<String>,
    /// Captures group positions only while building a dirty display list.
    indicator_stack: Vec<Vec<IndicatorBounds>>,
    cache_width: u32,
    pub commands: Vec<misa_kit::intent::Command>,
    pub notice: String,
    pub hits: Vec<Hit>,
    pub focus: Option<Control>,
    pub expanded: BTreeSet<String>,
    pub images: BTreeMap<String, Arc<image::RgbaImage>>,
    drafts: BTreeMap<(String, String), Editor>,
    field_viewports: BTreeMap<(String, String), FieldViewport>,
    save_viewport: FieldViewport,
    save: Option<(String, Editor)>,
    picker: Option<misa_kit::picker::Picker>,
    report: Option<Report>,
    selection: Option<((usize, usize), (usize, usize))>,
    rows: Vec<TextRow>,
    prefixes: Vec<(Style, String)>,
    replace_selection: bool,
    scroll: f32,
    follow: bool,
    content_height: f32,
    viewport_height: f32,
    /// Current elapsed-time pulse phase, not a repaint count.
    pub tick: u64,
    offline_elapsed: Duration,
}
struct Report {
    title: String,
    entries: Vec<String>,
    offset: usize,
    width: f32,
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
    pub fn new(view: Node, metrics: Arc<dyn TextMetrics>) -> Self {
        let mut app = Self {
            #[cfg(test)]
            rendered_nodes: 0,
            tree: IndexedTree::new(Node::section("session")),
            root: "session".into(),
            streams: BTreeMap::new(),
            cache: BTreeMap::new(),
            moving_indicators: BTreeSet::new(),
            indicator_stack: vec![],
            cache_width: 0,
            light: false,
            metrics,
            commands: vec![],
            notice: String::new(),
            hits: vec![],
            focus: None,
            expanded: BTreeSet::new(),
            images: BTreeMap::new(),
            drafts: BTreeMap::new(),
            field_viewports: BTreeMap::new(),
            save_viewport: FieldViewport::default(),
            save: None,
            picker: None,
            report: None,
            selection: None,
            rows: vec![],
            prefixes: vec![],
            replace_selection: false,
            scroll: 0.0,
            follow: true,
            content_height: 0.0,
            viewport_height: 600.0,
            tick: 0,
            offline_elapsed: Duration::ZERO,
        };
        app.set_view(view);
        app
    }
    /// Single backend-neutral input/paint driver. Hosts own effects and presentation.
    pub fn drive(&mut self, event: Event, elapsed: Duration) -> Output<Command, Scene> {
        let mut output = Output::default();
        match event {
            Event::Key(key) => {
                output.commands = self.key(key);
                output.redraw = true;
            }
            Event::Text(text) => {
                output.commands = self.text(&text);
                output.redraw = true;
            }
            Event::Pointer { x, y, dragging } => {
                output.commands = self.pointer(x, y, dragging);
                output.redraw = true;
            }
            Event::Wheel { delta } => {
                self.scroll(delta);
                output.redraw = true;
            }
            Event::Resize(_) => output.redraw = true,
            Event::Theme { light } => {
                self.set_light(light);
                output.redraw = true;
            }
            Event::Redraw(Size { width, height }) => {
                if width != 0 && height != 0 {
                    output.frame = Some(self.frame_at(width, height, elapsed));
                    if self.animating() {
                        output.deadline =
                            Some(misa_window_core::next_deadline(elapsed, PULSE_PERIOD));
                    }
                }
            }
        }
        output
    }

    pub fn set_light(&mut self, light: bool) {
        if self.light != light {
            self.light = light;
            self.cache.clear();
            self.moving_indicators.clear();
        }
    }
    fn colors(&self) -> crate::appearance::Palette {
        crate::appearance::Palette::new(self.light)
    }

    /// Only groups actually placed in the viewport by the last layout need a pulse.
    /// The root's bounds come from nested scene groups, so collapsed owners are absent.
    pub fn animating(&self) -> bool {
        self.visible_indicators().next().is_some()
    }

    fn visible_indicators(&self) -> impl Iterator<Item = &str> {
        let top = 20.0 - self.scroll;
        self.cache
            .get(&self.root)
            .into_iter()
            .flat_map(|cached| cached.indicators.iter())
            .filter(move |bounds| {
                top + bounds.top < self.viewport_height
                    && top + bounds.bottom > 0.0
                    && self.moving_indicators.contains(&bounds.id)
                    && self.cache.contains_key(&bounds.id)
            })
            .map(|bounds| bounds.id.as_str())
    }
    pub fn report(&mut self, title: String, value: Value) {
        let mut entries = Vec::new();
        Report::collect(&value, "", &mut entries);
        self.picker = None;
        self.report = Some(Report {
            title,
            entries,
            offset: 0,
            width: 0.0,
            lines: vec![],
        });
    }
    pub fn clear_secret_drafts(&mut self) {
        for ((node, field), edit) in &mut self.drafts {
            if self.tree.node(node).is_some_and(|node| matches!(&node.kind, Kind::Fields {fields} if fields.iter().any(|value| &value.id == field && value.secret))) { edit.set_text(""); }
        }
        self.cache.clear();
        self.moving_indicators.clear();
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
        self.moving_indicators.clear();
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
        self.field_viewports
            .retain(|key, _| self.drafts.contains_key(key));
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
            self.moving_indicators.remove(&id);
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
                self.moving_indicators.clear();
            }
        }
        for child in self.tree.children(id) {
            self.forget_subtree(&child);
        }
        self.cache.remove(id);
        self.moving_indicators.remove(id);
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
        self.field_viewports
            .retain(|key, _| self.drafts.contains_key(key));
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
    pub fn observed(&mut self, update: &DocumentUpdate<'_>) -> Result<(), String> {
        match update {
            DocumentUpdate::Reset { tree, streams } => {
                self.set_view((*tree).clone());
                self.reset_streams(streams);
            }
            DocumentUpdate::Changed {
                tree,
                live,
                reset_live,
            } => {
                if !tree.is_empty() {
                    self.apply_tree(*tree)?;
                }
                if *reset_live {
                    self.reset_streams(&[]);
                }
                for update in *live {
                    self.apply_stream(update);
                }
            }
            DocumentUpdate::Notice(message) => self.notice = message.to_string(),
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
            self.moving_indicators.clear();
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
    pub fn field_text(&self, node: &str, field: &str) -> Option<&str> {
        self.drafts
            .get(&(node.into(), field.into()))
            .map(Editor::text)
    }
    /// Pin an offline snapshot to the beginning of the view, rather than following
    /// live updates to the bottom. May be called before the first frame.
    pub fn pin_to_top(&mut self) {
        self.follow = false;
        self.scroll = 0.0;
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
            let column = self.rows.get(row).map(|row| row.column(x)).unwrap_or(0);
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
                self.save_viewport = FieldViewport::default();
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
    /// Committed text is handled separately from physical and special keys.
    fn text(&mut self, text: &str) -> Vec<Command> {
        self.invalidate_focus();
        let commands = self.text_inner(text);
        self.invalidate_focus();
        commands
    }
    fn text_inner(&mut self, text: &str) -> Vec<Command> {
        if self.report.is_some() {
            return vec![];
        }
        if let Some(picker) = &mut self.picker {
            for character in text.chars() {
                picker.type_char(character);
            }
            return vec![];
        }
        if let Some(control @ Control::Field { .. }) = self.focus.clone() {
            let discrete = match &control {
                Control::Field { node, field } => self.tree.node(node).is_some_and(|node| matches!(&node.kind, Kind::Fields { fields } if fields.iter().any(|value| &value.id == field && matches!(value.kind, FieldKind::Bool | FieldKind::Choice { .. })))),
                _ => false,
            };
            if discrete {
                if text == " " {
                    return self.activate(control);
                }
                return vec![];
            }
        }
        let replace = std::mem::take(&mut self.replace_selection);
        if let Some(edit) = self.editor() {
            if replace {
                edit.set_text("");
            }
            edit.insert(text);
        }
        vec![]
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
                self.commands
                    .iter()
                    .map(|command| misa_proto::view::Choice {
                        value: command.id.clone(),
                        label: command.label.clone(),
                        detail: Some(command.description.clone()),
                        metadata: None,
                    })
                    .collect(),
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
                if matches!(key, Key::Left | Key::Right) {
                    return self.activate(control);
                }
                return vec![];
            }
        }
        let replace = self.replace_selection;
        self.replace_selection = false;
        if let Some(edit) = self.editor() {
            match key {
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
                    text.geometry
                        .text
                        .chars()
                        .skip(from)
                        .take(to.saturating_sub(from))
                        .collect::<String>()
                })
            })
            .collect::<Vec<_>>()
            .join("\n")
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
const FONT_SIZE: f32 = 15.0;
fn text(x: f32, y: f32, value: &str, style: Style) -> Op {
    Op::Text {
        x,
        y,
        size: FONT_SIZE,
        style,
        text: value.into(),
    }
}
