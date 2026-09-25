//! Client-owned interaction and layout. No field draft, disclosure state or destination leaves
//! this module until the person activates an action the session advertised.
use misa_kit::editor::{Editor, Motion};
use misa_kit::intent::Intent;
use misa_pixel_ui::{Op, Scene, TextMetrics};
use misa_proto::sync::{StreamUpdate, ViewOp};
#[cfg(test)]
use misa_proto::view::FieldKind;
use misa_proto::view::{ActionOn, Kind, Node};
#[cfg(test)]
use misa_render::Theme;
use misa_style::Style;
use misa_value::Value;
pub use misa_window_core::Key;
use misa_window_core::{Event, Output, Size};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

mod document;
mod drafts;
mod interaction;
#[cfg(test)]
mod interaction_tests;
mod layout;
use interaction::Hit;
use interaction::{GroupGeometry, InteractionMap, PointerResult};
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
    geometry: GroupGeometry,
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
    document: document::DocumentStore,
    cache: BTreeMap<String, Arc<Cached>>,
    /// Painted status owners with a moving activity indicator (not document-wide turns).
    moving_indicators: BTreeSet<String>,
    /// Captures group positions only while building a dirty display list.
    indicator_stack: Vec<Vec<IndicatorBounds>>,
    cache_width: u32,
    pub commands: Vec<misa_kit::intent::Command>,
    pub notice: String,
    interaction: InteractionMap,
    pub expanded: BTreeSet<String>,
    drafts: drafts::Drafts,
    save_viewport: FieldViewport,
    save: Option<(String, Editor)>,
    picker: Option<misa_kit::picker::Picker>,
    report: Option<Report>,
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
            document: document::DocumentStore::new(Node::section("session")),
            cache: BTreeMap::new(),
            moving_indicators: BTreeSet::new(),
            indicator_stack: vec![],
            cache_width: 0,
            light: false,
            metrics,
            commands: vec![],
            notice: String::new(),
            interaction: InteractionMap::default(),
            expanded: BTreeSet::new(),
            drafts: drafts::Drafts::default(),
            save_viewport: FieldViewport::default(),
            save: None,
            picker: None,
            report: None,
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
            .get(self.document.root())
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
        for node in self.drafts.clear_secrets(&self.document) {
            self.invalidate(&node);
        }
    }
    pub fn reject_prompt(&mut self, text: String, reason: String) {
        if let Some(node) = self.drafts.recover_prompt(&text) {
            self.invalidate(&node);
        } else {
            // Keep newer typing intact. The separate local report can be copied
            // even if the original composer was removed by a scope update.
            self.report("Unsent prompt · Copy to recover".into(), Value::str(text));
        }
        self.notice = reason;
    }
    pub fn set_view(&mut self, view: Node) {
        self.set_view_with_streams(view, &[]);
    }
    fn set_view_with_streams(&mut self, mut view: Node, streams: &[misa_proto::sync::Stream]) {
        misa_proto::sync::address(&mut view);
        self.cache.clear();
        self.moving_indicators.clear();
        let keys = self.drafts.reset(&view);
        if self.save.is_some() {
            // A live update cannot steal focus from a local destination dialog.
        } else if let Some((node, field)) = keys.iter().find(|(node, _)| node == "panel.input" && self.document.node(node).is_none()) {
            if !matches!(self.interaction.focus(), Some(Control::Field { node: focused, .. }) if focused == node) {
                self.interaction.set_focus(Some(Control::Field { node: node.clone(), field: field.clone() }));
            }
        } else if self.interaction.focus().is_none_or(|control| matches!(control, Control::Field { node, field } if !self.drafts.contains(node, field))) {
            self.interaction.set_focus(keys.last().map(|(node,field)| Control::Field { node: node.clone(), field: field.clone() }));
        }
        let changes = self
            .document
            .observe(&DocumentUpdate::Reset {
                tree: &view,
                streams,
            })
            .expect("reset view is valid");
        self.invalidate_document(changes);
        self.interaction.clear_selection();
    }

    fn invalidate(&mut self, id: &str) {
        let mut cursor = Some(self.document.cache_owner(id).to_string());
        while let Some(id) = cursor {
            self.cache.remove(&id);
            self.moving_indicators.remove(&id);
            cursor = self.document.parent(&id).map(str::to_owned);
        }
    }
    fn invalidate_focus(&mut self) {
        let id = match self.interaction.focus() {
            Some(Control::Field { node, .. })
            | Some(Control::Action { node, .. })
            | Some(Control::Disclosure(node)) => Some(node.clone()),
            _ => None,
        };
        if let Some(id) = id {
            self.invalidate(&id);
        }
    }
    fn refresh_tree_fields(&mut self, operations: &[ViewOp]) {
        let panel_input = self.drafts.changed(operations, &self.document);
        if let Some((node, field)) = panel_input.filter(|_| self.save.is_none()) {
            self.interaction
                .set_focus(Some(Control::Field { node, field }));
        }
        if matches!(self.interaction.focus(), Some(Control::Field { node, field }) if !self.drafts.contains(node, field))
        {
            self.interaction.set_focus(None);
        }
        if self.interaction.focus().is_none() && self.save.is_none() {
            self.interaction.set_focus(
                self.drafts
                    .last()
                    .map(|(node, field)| Control::Field { node, field }),
            );
        }
    }
    fn invalidate_document(&mut self, changes: document::Changes) {
        if changes.full {
            self.cache.clear();
            self.moving_indicators.clear();
            return;
        }
        for id in changes.ids {
            self.invalidate(&id);
        }
    }
    /// One already-validated replica transaction. The window paints only after
    /// canonical changes and live retirement have both reached its derived cache.
    pub fn observed(&mut self, update: &DocumentUpdate<'_>) -> Result<(), String> {
        match update {
            DocumentUpdate::Reset { tree, streams } => {
                self.set_view_with_streams((*tree).clone(), streams);
            }
            DocumentUpdate::Changed { tree, .. } => {
                let changes = self.document.observe(update)?;
                if !tree.is_empty() {
                    self.refresh_tree_fields(tree);
                }
                self.invalidate_document(changes);
            }
            DocumentUpdate::Notice(message) => self.notice = message.to_string(),
        }
        Ok(())
    }
    /// Accept a decoder result only while its blob is still referenced by the document.
    pub fn image(&mut self, hash: String, image: Arc<image::RgbaImage>) {
        match self.document.image(hash, image) {
            document::ImageChange::Ignored => {}
            document::ImageChange::TooLarge => {
                self.notice = "Image exceeds the decoded cache limit".into()
            }
            document::ImageChange::Loaded(changes) => self.invalidate_document(changes),
        }
    }
    /// Inspect decoded-image retention without exposing the cache for mutation.
    pub fn has_image(&self, hash: &str) -> bool {
        self.document.has_image(hash)
    }
    /// Decoded bytes currently owned by the document (excludes display-list references).
    pub fn decoded_image_bytes(&self) -> usize {
        self.document.decoded_image_bytes()
    }
    pub fn field_text(&self, node: &str, field: &str) -> Option<&str> {
        self.drafts.text(node, field)
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
        match self.interaction.pointer(x, y, dragging) {
            PointerResult::None | PointerResult::SelectionChanged => vec![],
            PointerResult::Activate(control) => {
                self.follow = false;
                self.activate(control)
            }
        }
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
                self.drafts.cycle(&node, &field, &self.document);
            }
            Control::Action { node, action } if action == "attachment.save" => {
                self.save = Some((node, Editor::new()));
                self.save_viewport = FieldViewport::default();
                self.interaction.set_focus(Some(Control::SavePath));
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
                        self.interaction.set_focus(None);
                        return vec![command];
                    }
                }
            }
            Control::SaveCancel => {
                self.save = None;
                self.interaction.set_focus(None);
            }
            _ => {}
        }
        vec![]
    }
    fn submit(&mut self, node_id: &str, action_id: &str) -> Vec<Command> {
        let Some(node) = self.document.node(node_id) else {
            return vec![];
        };
        let Some(action) = node.actions.iter().find(|action| action.id == action_id) else {
            return vec![];
        };
        let mut fields = match &node.kind {
            Kind::Fields { fields } => fields.clone(),
            _ => vec![],
        };
        self.drafts.overlay(node_id, &mut fields);
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
            self.drafts.submit_composer(node_id);
        }
        vec![Command::Intent(intent)]
    }
    fn editor(&mut self) -> Option<&mut Editor> {
        match self.interaction.focus() {
            Some(Control::SavePath) => self.save.as_mut().map(|(_, edit)| edit),
            Some(Control::Field { node, field }) => self.drafts.editor_mut(node, field),
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
        if let Some(control @ Control::Field { .. }) = self.interaction.focus().cloned() {
            let discrete = match &control {
                Control::Field { node, field } => self.drafts.discrete(node, field, &self.document),
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
                        if let Some((node, field)) = self.drafts.insert_command(&candidate.value) {
                            self.invalidate(&node);
                            self.interaction
                                .set_focus(Some(Control::Field { node, field }));
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
            } else {
                self.interaction.select_all();
            }
            return vec![];
        }
        if let Key::Tab { backward } = key {
            self.interaction.next_focus(backward);
            return vec![];
        }
        if matches!(key, Key::Escape) {
            if self.save.take().is_some() {
                self.interaction.set_focus(None);
            } else {
                self.interaction.clear_selection();
            }
            return vec![];
        }
        if let Key::Enter { newline } = key {
            if self.interaction.focused(&Control::SavePath) {
                return self.activate(Control::SaveConfirm);
            }
            if let Some(Control::Field { node, field }) = self.interaction.focus().cloned() {
                if newline {
                    if let Some(edit) = self.editor() {
                        edit.insert("\n");
                    }
                    return vec![];
                }
                let action = self
                    .document
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
            if let Some(control) = self.interaction.focus().cloned() {
                return self.activate(control);
            }
        }
        if let Some(control @ Control::Field { .. }) = self.interaction.focus().cloned() {
            let discrete = match &control {
                Control::Field { node, field } => self.drafts.discrete(node, field, &self.document),
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
        self.interaction.selected_text()
    }
    /// Locate a control in the most recently painted frame.
    pub fn control_center(&self, control: &Control) -> Option<(f32, f32)> {
        self.interaction.control_center(control)
    }
    /// Restore local control focus (for example when a host restores a form).
    pub fn focus_control(&mut self, control: Option<Control>) {
        self.interaction.set_focus(control);
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
