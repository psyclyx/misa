//! Client-owned interaction and layout. No field draft, disclosure state or destination leaves
//! this module until the person activates an action the session advertised.
use misa_kit::editor::{Editor, Motion};
use misa_kit::intent::Intent;
use misa_pixel_ui::{
    ComboBox, ComboOption, ComboResult, ComboState, ContextMenu, FieldViewport, FlowViewport,
    MenuEntries, MenuEntry, MenuItem, MenuKey, MenuState, Op, Rect, Scene, TextMetrics,
};
use misa_proto::sync::{StreamUpdate, ViewOp};
#[cfg(test)]
use misa_proto::view::FieldKind;
use misa_proto::view::{ActionOn, Kind, Node};
use misa_render::Theme;
use misa_style::Style;
use misa_value::Value;
pub use misa_window_core::Key;
use misa_window_core::{Event, Output, Size};
use std::sync::Arc;
use std::time::Duration;

mod background;
mod document;
mod drafts;
mod flow;
#[cfg(test)]
mod flow_tests;
mod interaction;
#[cfg(test)]
mod interaction_tests;
mod layout;
mod measurement;
mod nodes;
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
    Scroll,
    LoadImage(misa_proto::view::BlobRef),
}
#[derive(Clone, Debug, PartialEq)]
enum MenuAction {
    Copy,
    SelectAll,
    Toggle(String),
}
pub(super) struct ChoiceRows<'a>(pub &'a [misa_proto::view::Choice]);
impl MenuEntries<String> for ChoiceRows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn entry(&self, i: usize) -> Option<MenuEntry<'_, String>> {
        self.0.get(i).map(|o| MenuEntry::Action {
            id: &o.value,
            label: &o.label,
            enabled: true,
        })
    }
}
struct DocumentMenu {
    anchor: (f32, f32),
    items: Vec<MenuItem<MenuAction>>,
    width: f32,
    state: MenuState,
    previous_focus: Option<Control>,
    field: Option<Control>,
}
pub struct DocumentUi {
    menu: Option<DocumentMenu>,
    choice: Option<(Control, ComboState)>,
    choice_options: Vec<ComboOption<String>>,
    size: Size,
    light: bool,
    metrics: Arc<dyn TextMetrics>,
    document: document::DocumentStore,
    retained: retained::RetainedScenes,
    overlays: LocalOverlays,
    interaction: InteractionMap,
    drafts: drafts::Drafts,
    viewport: FlowViewport<flow::FlowId>,
    offline_elapsed: Duration,
    background: Option<background::Background>,
}
impl DocumentUi {
    pub fn new(view: Node, metrics: Arc<dyn TextMetrics>) -> Self {
        let mut app = Self {
            menu: None,
            choice: None,
            choice_options: Vec::new(),
            size: Size {
                width: 0,
                height: 0,
            },
            document: document::DocumentStore::new(Node::section("session")),
            retained: retained::RetainedScenes::default(),
            light: false,
            metrics,
            overlays: LocalOverlays::default(),
            interaction: InteractionMap::default(),
            drafts: drafts::Drafts::default(),
            viewport: FlowViewport::default(),
            offline_elapsed: Duration::ZERO,
            background: None,
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
            Event::ContextMenu { x, y } => {
                self.choice = None;
                self.open_menu(x, y);
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
            Event::Resize(size) => {
                self.size = size;
                output.redraw = true;
            }
            Event::Theme { light } => {
                self.set_light(light);
                output.redraw = true;
            }
            Event::Redraw(Size { width, height }) => {
                if width != 0 && height != 0 {
                    output.frame = Some(self.frame_at(width, height, elapsed));
                    output.deadline = self.retained.next_deadline(elapsed);
                }
            }
        }
        output
    }

    pub fn set_light(&mut self, light: bool) {
        if self.light != light {
            self.light = light;
            self.retained.clear();
            self.cancel_background(false);
        }
    }
    /// The theme every layout path measures against.
    pub(super) fn theme(&self) -> Theme {
        if self.light {
            Theme::light()
        } else {
            Theme::dark()
        }
    }
    /// Any content change can move an owner's height: results measured against
    /// the previous content may never be installed.
    fn cancel_background(&mut self, restart: bool) {
        if let Some(background) = &mut self.background {
            background.cancel(restart);
        }
    }
    /// Prewarm exact owner measurements on one protocol-free worker. The waker
    /// may be called from that worker and must only request a UI poll; layout
    /// results never enter the connection-update path.
    pub fn enable_background(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        match &mut self.background {
            Some(background) => background.resume(waker),
            None => {
                self.background = Some(background::Background::start(self.metrics.clone(), waker));
            }
        }
    }
    /// Stop prewarming and stop waking the host. In-flight work drains silently.
    pub fn pause_background(&mut self) {
        if let Some(background) = &mut self.background {
            background.pause();
        }
    }
    /// Advance bounded prewarm work on the UI thread. Installing an offscreen
    /// height or display list never changes the current frame.
    pub fn poll_background(&mut self) -> usize {
        let Some(mut background) = self.background.take() else {
            return 0;
        };
        let installed = background::poll(&mut background, self);
        self.background = Some(background);
        installed
    }
    /// Whether prewarm work remains: queued results, or an unexhausted cursor.
    pub fn background_work_pending(&self) -> bool {
        self.background
            .as_ref()
            .is_some_and(background::Background::work_pending)
    }
    /// Only painted moving groups intersecting the viewport need a pulse.
    pub fn animating(&self) -> bool {
        self.retained.animating()
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
        self.drafts.focus_changed(self.interaction.focus());
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
        // A reset can replace every reading ID. Capture its semantic place before
        // replacing the index; ordinary edits and tail following remain untouched.
        let theme = if self.light {
            Theme::light()
        } else {
            Theme::dark()
        };
        let anchor = flow::ResetAnchor::capture(&self.document, &self.viewport, &theme);
        self.menu = None;
        self.choice = None;
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
        if let Some(anchor) = anchor {
            anchor.restore(&self.document, &mut self.viewport, &theme);
        }
        self.invalidate_document(changes);
        self.interaction.clear_selection();
        self.drafts.focus_changed(self.interaction.focus());
    }

    fn invalidate(&mut self, id: &str) {
        self.cancel_background(false);
        if let Some((owner, index)) = self.document.fragment_owner(id) {
            self.viewport
                .invalidate(&flow::FlowId::Row(owner.to_owned(), index));
        }
        // List-item fields are embedded in their indexed list owner.
        let mut cursor = Some(self.document.cache_owner(id));
        while let Some(owner) = cursor {
            self.viewport
                .invalidate(&flow::FlowId::Node(owner.to_owned()));
            self.viewport
                .invalidate(&flow::FlowId::Stream(owner.to_owned()));
            cursor = self.document.parent(owner);
        }
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
        self.drafts.focus_changed(self.interaction.focus());
    }
    fn invalidate_document(&mut self, changes: document::Changes) {
        self.cancel_background(changes.full);
        if changes.full {
            self.interaction.clear_selection();
        }
        self.interaction.retire(&changes.retired);
        if changes.full {
            self.viewport.clear_measurements();
        } else {
            // Retire every measured fragment for a changed or removed owner,
            // including owners already absent from the canonical index.
            for id in &changes.ids {
                for key in self.retained.row_keys(id) {
                    if let Some(index) = key
                        .strip_prefix("\0row:")
                        .and_then(|rest| rest.rsplit_once(':'))
                        .and_then(|(_, row)| row.parse().ok())
                    {
                        self.viewport
                            .invalidate(&flow::FlowId::Row(id.clone(), index));
                    } else {
                        self.viewport.invalidate(&flow::FlowId::End(id.clone()));
                    }
                }
            }
            for id in &changes.end_ids {
                self.viewport.invalidate(&flow::FlowId::End(id.clone()));
            }
            for id in &changes.ids {
                let mut cursor = Some(id.as_str());
                while let Some(owner) = cursor {
                    self.viewport
                        .invalidate(&flow::FlowId::Node(owner.to_owned()));
                    self.viewport
                        .invalidate(&flow::FlowId::Stream(owner.to_owned()));
                    cursor = self.document.parent(owner);
                }
            }
        }
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
                // Only a transaction touching the popup's field owner can invalidate its rows.
                if let Some((Control::Field { node, field }, _)) = &self.choice {
                    if changes.full
                        || changes.retired.contains(node)
                        || !self.document.field(node, field).is_some_and(|f| {
                            matches!(f.kind, misa_proto::view::FieldKind::Choice { .. })
                                && !f.read_only
                                && !f.secret
                        })
                    {
                        self.choice = None;
                    }
                }
                if !tree.is_empty() {
                    self.menu = None;
                }
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
        self.viewport.anchor(flow::FlowId::Top, 0.0, 0.0);
        if self.viewport.constraints().height > 0.0 {
            let mut builder = layout::LayoutBuilder::new(
                &self.document,
                &mut self.drafts,
                &mut self.interaction,
                &mut self.retained,
                &mut self.overlays,
                self.metrics.as_ref(),
                self.light,
            );
            let constraints = self.viewport.constraints();
            self.viewport.layout(&mut builder, constraints);
            self.sync_visible_moving();
        }
    }
    /// Refresh pulse bounds from the flow's current placements without composing
    /// a scene or applying editor viewport effects on a wheel event.
    fn sync_visible_moving(&mut self) {
        let constraints = self.viewport.constraints();
        self.retained.begin_placement(constraints.height);
        for placement in self.viewport.visible() {
            if matches!(
                placement.id,
                flow::FlowId::Node(_)
                    | flow::FlowId::Stream(_)
                    | flow::FlowId::Row(_, _)
                    | flow::FlowId::End(_)
            ) {
                let key = placement.id.cache_key();
                if let Some(cached) = self.retained.get(&key, constraints.width) {
                    self.retained.place(&key, &cached, placement.y);
                }
            }
        }
    }
    fn choice_widget(
        &self,
        control: &Control,
    ) -> Option<misa_pixel_ui::PlacedComboBox<'_, String>> {
        let Control::Field { node, field } = control else {
            return None;
        };
        let model = self.document.field(node, field)?;
        if !matches!(model.kind, misa_proto::view::FieldKind::Choice { .. })
            || model.read_only
            || model.secret
        {
            return None;
        }
        let bounds = self.interaction.control_bounds(control)?;
        // The option model is built on opening, not per frame.
        let selected = self
            .drafts
            .text(node, field)
            .unwrap_or(&model.value)
            .to_string();
        let colors = crate::appearance::Palette::new(self.light);
        Some(
            ComboBox {
                options: &self.choice_options,
                selected: Some(&selected),
                bounds,
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: self.size.width as f32,
                    height: self.size.height as f32,
                },
                font_size: FONT_SIZE,
                row_height: 28.0,
                focused: self.interaction.focused(control),
                enabled: true,
                background: colors.field,
                foreground: colors.text,
                muted: colors.muted,
                border: colors.border,
                highlight: colors.accent,
            }
            .place(self.metrics.as_ref()),
        )
    }
    fn open_choice(&mut self, control: Control, key: MenuKey) -> bool {
        if let Control::Field { node, field } = &control {
            if let Some(model) = self.document.field(node, field)
                && let misa_proto::view::FieldKind::Choice { options, .. } = &model.kind
                && !model.secret
                && !model.read_only
            {
                self.choice_options = options
                    .iter()
                    .map(|o| ComboOption {
                        value: o.value.clone(),
                        label: o.label.clone(),
                        enabled: true,
                    })
                    .collect();
            }
        }
        let Some(widget) = self.choice_widget(&control) else {
            return false;
        };
        let mut state = ComboState::default();
        let result = widget.key(self.metrics.as_ref(), &mut state, key);
        self.finish_choice(control, state, result);
        true
    }
    fn finish_choice(&mut self, control: Control, state: ComboState, result: ComboResult<String>) {
        if let ComboResult::Selected(value) = &result {
            if let Control::Field { node, field } = &control {
                self.drafts.set_choice(node, field, value);
                self.invalidate(node);
            }
        }
        if state.open {
            self.choice = Some((control, state));
        }
    }
    fn menu_widget<'a>(
        &self,
        items: &'a Vec<MenuItem<MenuAction>>,
        anchor: (f32, f32),
    ) -> ContextMenu<'a, MenuAction> {
        let colors = crate::appearance::Palette::new(self.light);
        ContextMenu {
            items,
            anchor,
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: self.size.width as f32,
                height: self.size.height as f32,
            },
            font_size: FONT_SIZE,
            row_height: 28.0,
            background: colors.surface,
            foreground: colors.text,
            muted: colors.muted,
            highlight: colors.accent,
        }
    }
    fn open_menu(&mut self, x: f32, y: f32) {
        if self.overlays.pointer_blocked() || self.overlays.saving() {
            return;
        }
        let target = self.interaction.hit_at(x, y);
        let mut items = Vec::new();
        let mut field = None;
        match target {
            Some(control @ Control::Field { .. }) => {
                if let Control::Field { node, field: name } = &control {
                    if self.drafts.contains(node, name)
                        && !self.drafts.discrete(node, name, &self.document)
                    {
                        let content = self.drafts.text(node, name).unwrap_or("");
                        items.push(MenuItem::Action {
                            id: MenuAction::Copy,
                            label: "Copy field".into(),
                            enabled: !content.is_empty()
                                && !self.document.field(node, name).is_some_and(|f| f.secret),
                        });
                        items.push(MenuItem::Action {
                            id: MenuAction::SelectAll,
                            label: "Select all in field".into(),
                            enabled: !content.is_empty(),
                        });
                        field = Some(control);
                    }
                }
            }
            Some(Control::Disclosure(id)) => {
                let label = if self.interaction.is_expanded(&id) {
                    "Collapse"
                } else {
                    "Expand"
                };
                items.push(MenuItem::Action {
                    id: MenuAction::Toggle(id),
                    label: label.into(),
                    enabled: true,
                });
            }
            _ => {
                let selected = self.selected_text();
                items.push(MenuItem::Action {
                    id: MenuAction::Copy,
                    label: "Copy selection".into(),
                    enabled: !selected.is_empty(),
                });
                items.push(MenuItem::Action {
                    id: MenuAction::SelectAll,
                    label: "Select all text".into(),
                    enabled: true,
                });
            }
        }
        if !items.is_empty() {
            self.menu = Some(DocumentMenu {
                anchor: (x, y),
                width: self
                    .menu_widget(&items, (x, y))
                    .measured_width(self.metrics.as_ref()),
                items,
                state: MenuState::default(),
                previous_focus: self.interaction.focus().cloned(),
                field,
            });
        }
    }
    fn finish_menu(
        &mut self,
        menu: DocumentMenu,
        result: Option<Option<MenuAction>>,
    ) -> Vec<Command> {
        let Some(action) = result else {
            self.menu = Some(menu);
            return vec![];
        };
        self.interaction.set_focus(menu.previous_focus);
        self.drafts.focus_changed(self.interaction.focus());
        match action {
            Some(MenuAction::Copy) => {
                let text = match menu.field {
                    Some(Control::Field { node, field }) => {
                        self.drafts.text(&node, &field).unwrap_or("").to_owned()
                    }
                    _ => self.selected_text(),
                };
                if text.is_empty() {
                    vec![]
                } else {
                    vec![Command::Copy(text)]
                }
            }
            Some(MenuAction::SelectAll) => {
                if let Some(Control::Field { node, field }) = menu.field {
                    self.interaction.set_focus(Some(Control::Field {
                        node: node.clone(),
                        field: field.clone(),
                    }));
                    self.drafts.select_all(&node, &field);
                } else {
                    self.interaction.select_all();
                }
                vec![]
            }
            Some(MenuAction::Toggle(id)) => {
                self.anchor_action_at(menu.anchor.1);
                self.invalidate(&id);
                self.interaction.toggle_disclosure(&id);
                vec![]
            }
            None => vec![],
        }
    }
    pub fn scroll(&mut self, delta: f32) {
        if let Some((control, mut state)) = self.choice.take() {
            if let Some(widget) = self.choice_widget(&control) {
                widget.wheel(self.metrics.as_ref(), &mut state, delta);
                self.choice = Some((control, state));
            }
            return;
        }
        if let Some(mut menu) = self.menu.take() {
            let widget = self.menu_widget(&menu.items, menu.anchor);
            let placed =
                widget.place_with_width(self.metrics.as_ref(), &mut menu.state, menu.width);
            placed.wheel(&mut menu.state, delta);
            self.menu = Some(menu);
            return;
        }
        if self.overlays.scroll(delta) {
            return;
        }
        let mut builder = layout::LayoutBuilder::new(
            &self.document,
            &mut self.drafts,
            &mut self.interaction,
            &mut self.retained,
            &mut self.overlays,
            self.metrics.as_ref(),
            self.light,
        );
        self.viewport.wheel(&mut builder, delta);
        self.sync_visible_moving();
    }
    pub fn pointer(&mut self, x: f32, y: f32, dragging: bool) -> Vec<Command> {
        if let Some((control, mut state)) = self.choice.take() {
            if !dragging {
                if let Some(widget) = self.choice_widget(&control) {
                    let result = widget.click(self.metrics.as_ref(), &mut state, x, y);
                    self.finish_choice(control, state, result);
                }
            } else {
                self.choice = Some((control, state));
            }
            return vec![];
        }
        if self.menu.is_some() {
            if dragging {
                return vec![];
            }
            let mut menu = self.menu.take().unwrap();
            let result = self
                .menu_widget(&menu.items, menu.anchor)
                .place_with_width(self.metrics.as_ref(), &mut menu.state, menu.width)
                .click(x, y);
            return self.finish_menu(menu, result);
        }
        if self.overlays.pointer_blocked() {
            return vec![];
        }
        self.invalidate_focus();
        let commands = self.pointer_inner(x, y, dragging);
        self.invalidate_focus();
        self.drafts.focus_changed(self.interaction.focus());
        commands
    }
    fn pointer_inner(&mut self, x: f32, y: f32, dragging: bool) -> Vec<Command> {
        match self.interaction.pointer(x, y, dragging) {
            PointerResult::None | PointerResult::SelectionChanged => vec![],
            PointerResult::Activate(control) => {
                if matches!(control, Control::Scroll) {
                    self.scroll_drag(y);
                    return vec![];
                }
                self.anchor_action_at(y);
                self.activate(control)
            }
        }
    }

    /// The transcript's scrollbar, once the measured heights cover the whole
    /// source: a thumb that lies about where the reader is costs more than
    /// none at all.
    fn scrollbar(&mut self) -> Option<misa_pixel_ui::PlacedScrollbar<Control>> {
        if !self
            .background
            .as_ref()
            .is_some_and(background::Background::sweep_complete)
        {
            return None;
        }
        let mut builder = layout::LayoutBuilder::new(
            &self.document,
            &mut self.drafts,
            &mut self.interaction,
            &mut self.retained,
            &mut self.overlays,
            self.metrics.as_ref(),
            self.light,
        );
        let scroll = self.viewport.scroll(&mut builder)?;
        let colors = crate::appearance::Palette::new(self.light);
        Some(
            misa_pixel_ui::Scrollbar {
                id: Control::Scroll,
                bounds: misa_pixel_ui::Rect {
                    x: self.size.width as f32 - 9.0,
                    y: 0.0,
                    width: 6.0,
                    height: self.viewport.constraints().height,
                },
                scroll,
                track: colors.border,
                thumb_style: colors.muted,
            }
            .place(),
        )
    }

    /// Dragging the thumb asks for the content offset under it. The walk is
    /// bounded per event: a drag converges over its events, never in one.
    fn scroll_drag(&mut self, y: f32) {
        let Some(bar) = self.scrollbar() else {
            return;
        };
        let target = bar.drag(y);
        let mut builder = layout::LayoutBuilder::new(
            &self.document,
            &mut self.drafts,
            &mut self.interaction,
            &mut self.retained,
            &mut self.overlays,
            self.metrics.as_ref(),
            self.light,
        );
        self.viewport.scroll_to(&mut builder, target, SCROLL_BUDGET);
    }
    fn anchor_action_at(&mut self, y: f32) {
        if matches!(
            self.viewport.position,
            misa_pixel_ui::FlowPosition::FollowTail
        ) {
            // Height-changing actions keep the acted-on owner fixed, not the
            // later tail. Menus use the original right-click point here too.
            if let Some(clicked) = self
                .viewport
                .visible()
                .iter()
                .find(|p| {
                    matches!(
                        p.id,
                        flow::FlowId::Node(_) | flow::FlowId::Stream(_) | flow::FlowId::Row(_, _)
                    ) && y >= p.y
                        && y < p.y + p.height
                })
                .cloned()
            {
                self.viewport.anchor(clicked.id, y - clicked.y, y);
            } else {
                self.viewport.anchor(flow::FlowId::Top, 0.0, 0.0);
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
            Control::Disclosure(id) => self.interaction.toggle_disclosure(&id),
            Control::Field { node, field } => {
                let control = Control::Field {
                    node: node.clone(),
                    field: field.clone(),
                };
                if !self.open_choice(control, MenuKey::Enter) {
                    self.drafts.cycle(&node, &field, &self.document);
                }
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
        if let Some((control, mut state)) = self.choice.take() {
            let mapped = match key {
                Key::Escape => Some(MenuKey::Escape),
                Key::Up => Some(MenuKey::Up),
                Key::Down => Some(MenuKey::Down),
                Key::Home => Some(MenuKey::Home),
                Key::End => Some(MenuKey::End),
                Key::Enter { .. } => Some(MenuKey::Enter),
                _ => None,
            };
            if let Some(widget) = self.choice_widget(&control) {
                let result = mapped.map_or(ComboResult::Handled, |k| {
                    widget.key(self.metrics.as_ref(), &mut state, k)
                });
                self.finish_choice(control, state, result);
            }
            return vec![];
        }
        if self.menu.is_some() {
            let mut menu = self.menu.take().unwrap();
            let result = if let Some(k) = match key {
                Key::Escape => Some(MenuKey::Escape),
                Key::Up => Some(MenuKey::Up),
                Key::Down => Some(MenuKey::Down),
                Key::Home => Some(MenuKey::Home),
                Key::End => Some(MenuKey::End),
                Key::Enter { .. } => Some(MenuKey::Enter),
                _ => None,
            } {
                self.menu_widget(&menu.items, menu.anchor)
                    .place_with_width(self.metrics.as_ref(), &mut menu.state, menu.width)
                    .key(&mut menu.state, k)
            } else {
                None
            };
            return self.finish_menu(menu, result);
        }
        if key == Key::Menu {
            if !self.overlays.pointer_blocked() && !self.overlays.saving() {
                let (x, y) = self
                    .interaction
                    .focus()
                    .and_then(|c| self.interaction.control_center(c))
                    .unwrap_or((20.0, 20.0));
                self.open_menu(x, y);
                if self.menu.is_none() && !self.selected_text().is_empty() {
                    self.open_menu(20.0, 20.0);
                }
                if let Some(menu) = &mut self.menu {
                    menu.state.highlighted = menu
                        .items
                        .iter()
                        .position(|item| matches!(item, MenuItem::Action { enabled: true, .. }));
                }
            }
            return vec![];
        }
        self.invalidate_focus();
        let commands = self.key_inner(key);
        self.invalidate_focus();
        self.drafts.focus_changed(self.interaction.focus());
        commands
    }
    /// Committed text is handled separately from physical and special keys.
    fn text(&mut self, text: &str) -> Vec<Command> {
        if self.menu.is_some() || self.choice.is_some() {
            return vec![];
        }
        self.invalidate_focus();
        let commands = self.text_inner(text);
        self.invalidate_focus();
        self.drafts.focus_changed(self.interaction.focus());
        commands
    }
    fn text_inner(&mut self, text: &str) -> Vec<Command> {
        let decision = self.overlays.text(text, self.interaction.focus());
        if let Some(commands) = self.overlay_decision(decision) {
            self.drafts.clear_selection();
            return commands;
        }
        if let Some(control @ Control::Field { .. }) = self.interaction.focus().cloned() {
            let discrete = match &control {
                Control::Field { node, field } => self.drafts.discrete(node, field, &self.document),
                _ => false,
            };
            if discrete {
                self.drafts.clear_selection();
                if text == " " {
                    return self.activate(control);
                }
                return vec![];
            }
        }
        if let Some(Control::Field { node, field }) = self.interaction.focus().cloned() {
            self.drafts.insert(&node, &field, text);
        } else {
            self.drafts.clear_selection();
        }
        vec![]
    }
    fn key_inner(&mut self, key: Key) -> Vec<Command> {
        let decision = self.overlays.key(key.clone(), self.interaction.focus());
        if let Some(commands) = self.overlay_decision(decision) {
            self.drafts.clear_selection();
            return commands;
        }
        if !matches!(key, Key::SelectAll | Key::Backspace) {
            self.drafts.clear_selection();
        }
        if matches!(key, Key::Copy) {
            if matches!(self.interaction.focus(), Some(Control::Field { node, field })
                if self.document.field(node, field).is_some_and(|value| value.secret))
            {
                return vec![];
            }
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
            if let Some(Control::Field { node, field }) = self.interaction.focus().cloned()
                && self.drafts.contains(&node, &field)
            {
                if self.drafts.discrete(&node, &field, &self.document) {
                    self.drafts.clear_selection();
                } else {
                    self.drafts.select_all(&node, &field);
                }
            } else {
                self.drafts.clear_selection();
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
        if let Some(control @ Control::Field { .. }) = self.interaction.focus().cloned() {
            let combo_key = match key {
                Key::Up => Some(MenuKey::Up),
                Key::Down => Some(MenuKey::Down),
                Key::Home => Some(MenuKey::Home),
                Key::End => Some(MenuKey::End),
                Key::Enter { newline: false } => Some(MenuKey::Enter),
                _ => None,
            };
            if let Some(k) = combo_key
                && self.open_choice(control, k)
            {
                return vec![];
            }
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
        if matches!(key, Key::Backspace) {
            if let Some(Control::Field { node, field }) = self.interaction.focus().cloned() {
                self.drafts.backspace(&node, &field);
            } else {
                self.drafts.clear_selection();
            }
            return vec![];
        }
        if let Some(edit) = self.editor() {
            match key {
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
        self.drafts.focus_changed(self.interaction.focus());
    }
}

const FONT_SIZE: f32 = 16.0;

/// Block chrome shared by every owner: how much room a card keeps off its
/// content, the gap one carded block leaves for the next, and one level of
/// quote/disclosure indent. These are drawn, never glyphs: a font can lack a
/// box-drawing character, but not a rectangle.
pub(super) const CARD_PADDING_X: f32 = 8.0;
pub(super) const CARD_PADDING_Y: f32 = 6.0;
/// Content to content across two cards: both paddings plus the gap between.
pub(super) const CARD_TRAILING: f32 = 2.0 * CARD_PADDING_Y + 6.0;
/// Undecorated blocks keep the tighter paragraph rhythm.
pub(super) const PARAGRAPH_GAP: f32 = 5.0;
/// Owners one drag step walks toward its target before the next event.
pub(super) const SCROLL_BUDGET: usize = 256;
pub(super) const GUTTER: f32 = 12.0;
pub(super) const RAIL: f32 = 2.0;
fn text(x: f32, y: f32, value: &str, style: Style) -> Op {
    Op::Text {
        x,
        y,
        size: FONT_SIZE,
        style,
        text: value.into(),
    }
}
