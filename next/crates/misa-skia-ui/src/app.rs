//! Client-owned interaction and layout. No field draft, disclosure state or destination leaves
//! this module until the person activates an action the session advertised.
use misa_kit::editor::{Editor, Motion};
use misa_kit::intent::Intent;
use misa_pixel_ui::{Op, Scene, TextMetrics, Viewport};
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
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

mod document;
mod drafts;
mod interaction;
#[cfg(test)]
mod interaction_tests;
mod layout;
mod overlays;
mod retained;
use interaction::Hit;
use interaction::{InteractionMap, PointerResult};
use overlays::{Decision, LocalOverlays, OverlayAction};
#[cfg(test)]
mod tests;
mod text;

/// Pulse cadence shared by the fake clock and the window scheduler.
pub const PULSE_PERIOD: Duration = Duration::from_millis(160);

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
pub struct App {
    light: bool,
    metrics: Arc<dyn TextMetrics>,
    document: document::DocumentStore,
    retained: retained::RetainedScenes,
    overlays: LocalOverlays,
    interaction: InteractionMap,
    pub expanded: BTreeSet<String>,
    drafts: drafts::Drafts,
    prefixes: Vec<(Style, String)>,
    replace_selection: bool,
    viewport: Viewport,
    offline_elapsed: Duration,
}
impl App {
    pub fn new(view: Node, metrics: Arc<dyn TextMetrics>) -> Self {
        let mut app = Self {
            document: document::DocumentStore::new(Node::section("session")),
            retained: retained::RetainedScenes::default(),
            light: false,
            metrics,
            overlays: LocalOverlays::default(),
            interaction: InteractionMap::default(),
            expanded: BTreeSet::new(),
            drafts: drafts::Drafts::default(),
            prefixes: vec![],
            replace_selection: false,
            viewport: Viewport::new(0.0, 600.0),
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
                    output.deadline = self.retained.next_deadline(
                        self.document.root(),
                        self.viewport.offset(),
                        self.viewport.viewport_height(),
                        elapsed,
                    );
                }
            }
        }
        output
    }

    pub fn set_light(&mut self, light: bool) {
        if self.light != light {
            self.light = light;
            self.retained.clear();
        }
    }
    fn colors(&self) -> crate::appearance::Palette {
        crate::appearance::Palette::new(self.light)
    }

    /// Only painted moving groups intersecting the viewport need a pulse.
    pub fn animating(&self) -> bool {
        self.retained.animating(
            self.document.root(),
            self.viewport.offset(),
            self.viewport.viewport_height(),
        )
    }
    pub fn report(&mut self, title: String, value: Value) {
        self.overlays.report(title, value);
    }
    pub fn notice(&mut self, text: &str) {
        self.overlays.notice(text);
    }
    pub fn notice_text(&self) -> &str {
        self.overlays.notice_text()
    }
    pub fn declare_commands(&mut self, commands: Vec<misa_kit::intent::Command>) {
        self.overlays.declare_commands(commands);
    }
    fn overlay_decision(&mut self, decision: Decision) -> Option<Vec<Command>> {
        let Decision::Consumed {
            command,
            focus,
            action,
        } = decision
        else {
            return None;
        };
        if let Some(focus) = focus {
            self.interaction.set_focus(focus);
        }
        if let Some(OverlayAction::InsertCommand(value)) = action {
            let target = self.drafts.insert_command(&value);
            self.overlays.inserted(target.is_some());
            if let Some((node, field)) = target {
                self.invalidate(&node);
                self.interaction
                    .set_focus(Some(Control::Field { node, field }));
            }
        }
        Some(command.into_iter().collect())
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
        self.notice(&reason);
    }
    pub fn set_view(&mut self, view: Node) {
        self.set_view_with_streams(view, &[]);
    }
    fn set_view_with_streams(&mut self, mut view: Node, streams: &[misa_proto::sync::Stream]) {
        misa_proto::sync::address(&mut view);
        let keys = self.drafts.reset(&view);
        if self.overlays.saving() {
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
        self.retained.invalidate(id, &self.document);
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
        if let Some((node, field)) = panel_input.filter(|_| !self.overlays.saving()) {
            self.interaction
                .set_focus(Some(Control::Field { node, field }));
        }
        if matches!(self.interaction.focus(), Some(Control::Field { node, field }) if !self.drafts.contains(node, field))
        {
            self.interaction.set_focus(None);
        }
        if self.interaction.focus().is_none() && !self.overlays.saving() {
            self.interaction.set_focus(
                self.drafts
                    .last()
                    .map(|(node, field)| Control::Field { node, field }),
            );
        }
    }
    fn invalidate_document(&mut self, changes: document::Changes) {
        self.retained.invalidate_document(changes, &self.document);
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
            DocumentUpdate::Notice(message) => self.notice(message),
        }
        Ok(())
    }
    /// Accept a decoder result only while its blob is still referenced by the document.
    pub fn image(&mut self, hash: String, image: Arc<image::RgbaImage>) {
        match self.document.image(hash, image) {
            document::ImageChange::Ignored => {}
            document::ImageChange::TooLarge => self.notice("Image exceeds the decoded cache limit"),
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
        self.viewport.pin_to_top();
    }
    pub fn scroll(&mut self, delta: f32) {
        if self.overlays.scroll(delta) {
            return;
        }
        self.viewport.scroll(delta);
    }
    pub fn pointer(&mut self, x: f32, y: f32, dragging: bool) -> Vec<Command> {
        if self.overlays.pointer_blocked() {
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
                self.viewport.stop_following();
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
                let decision = self.overlays.open_save(node);
                return self.overlay_decision(decision).unwrap_or_default();
            }
            Control::Action { node, action } => return self.submit(&node, &action),
            Control::SaveConfirm | Control::SaveCancel => {
                let decision = self.overlays.activate(&control);
                return self.overlay_decision(decision).unwrap_or_default();
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
            let Some(intent) = self.overlays.parse(text) else {
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
        let decision = self.overlays.text(text, self.interaction.focus());
        if let Some(commands) = self.overlay_decision(decision) {
            return commands;
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
        let decision = self.overlays.key(key.clone(), self.interaction.focus());
        if let Some(commands) = self.overlay_decision(decision) {
            return commands;
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
            self.interaction.clear_selection();
            return vec![];
        }
        if let Key::Enter { newline } = key {
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
