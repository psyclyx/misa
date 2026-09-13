//! Client-owned interaction and layout. No field draft, disclosure state or destination leaves
//! this module until the person activates an action the session advertised.
use crate::{Layout, Op, Scene};
use misa_client::editor::{Editor, Motion};
use misa_proto::view::{ActionOn, FieldKind, Kind, Node};
use misa_proto::wire::{Intent, SessionInfo};
use misa_render::{Color, Style, Theme};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Intent(Intent),
    Save { node: String, destination: String },
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
    text: String,
}
#[derive(Clone, Debug)]
pub enum Key {
    Text(String),
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
    pub view: Node,
    pub info: Option<SessionInfo>,
    pub notice: String,
    pub hits: Vec<Hit>,
    pub focus: Option<Control>,
    pub expanded: BTreeSet<String>,
    pub images: BTreeMap<String, Arc<image::RgbaImage>>,
    drafts: BTreeMap<(String, String), Editor>,
    save: Option<(String, Editor)>,
    selection: Option<((usize, usize), (usize, usize))>,
    rows: Vec<TextRow>,
    replace_selection: bool,
    scroll: f32,
    follow: bool,
    content_height: f32,
    viewport_height: f32,
}
impl App {
    pub fn new(view: Node) -> Self {
        let mut app = Self {
            view: Node::section("session"),
            info: None,
            notice: String::new(),
            hits: vec![],
            focus: None,
            expanded: BTreeSet::new(),
            images: BTreeMap::new(),
            drafts: BTreeMap::new(),
            save: None,
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
    pub fn set_view(&mut self, view: Node) {
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
        if let Some((node, field)) = keys.iter().find(|(node, _)| node == "panel.input") {
            if !matches!(&self.focus, Some(Control::Field { node: focused, .. }) if focused == node) {
                self.focus = Some(Control::Field { node: node.clone(), field: field.id.clone() });
            }
        } else if self.focus.as_ref().is_none_or(|control| matches!(control, Control::Field { node, field } if !self.drafts.contains_key(&(node.clone(),field.clone())))) {
            self.focus = keys.last().map(|(node,field)| Control::Field { node: node.clone(), field: field.id.clone() });
        }
        self.view = view;
        self.selection = None;
    }
    pub fn field_text(&self, node: &str, field: &str) -> Option<&str> {
        self.drafts
            .get(&(node.into(), field.into()))
            .map(Editor::text)
    }
    pub fn scroll(&mut self, delta: f32) {
        self.scroll =
            (self.scroll + delta).clamp(0.0, (self.content_height - self.viewport_height).max(0.0));
        self.follow = false;
    }
    pub fn pointer(&mut self, x: f32, y: f32, dragging: bool) -> Vec<Command> {
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
            let column = ((x - hit.x).max(0.0) / Layout::default().advance).floor() as usize;
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
        match control {
            Control::Disclosure(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Control::Field { node, field } => {
                let kind =
                    misa_proto::view::find(&self.view, &node).and_then(|node| match &node.kind {
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
        let Some(node) = misa_proto::view::find(&self.view, node_id) else {
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
            let commands = self
                .info
                .as_ref()
                .map(|info| info.commands.as_slice())
                .unwrap_or(&[]);
            let parsed = misa_client::intent::parse(text, commands);
            let Some(intent) = misa_client::intent::intent(&parsed) else {
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
        if action.on == ActionOn::Submit {
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
                let action = misa_proto::view::find(&self.view, &node)
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
                Control::Field {node,field} => misa_proto::view::find(&self.view,node).is_some_and(|node| matches!(&node.kind, Kind::Fields {fields} if fields.iter().any(|value| &value.id == field && matches!(value.kind,FieldKind::Bool|FieldKind::Choice {..})))),
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
        self.viewport_height = height as f32;
        let scene = self.layout(width, height);
        let max = (self.content_height - height as f32 + 40.0).max(0.0);
        let wanted = if self.follow {
            max
        } else {
            self.scroll.min(max)
        };
        if (wanted - self.scroll).abs() > 0.5 {
            self.scroll = wanted;
            return self.layout(width, height);
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
        let view = self.view.clone();
        let theme = Theme::dark();
        let mut y = 20.0 - self.scroll;
        self.node(
            &view,
            20.0,
            &mut y,
            (width as f32 - 40.0).max(40.0),
            &theme,
            &mut scene,
        );
        self.content_height = y + self.scroll + 20.0;
        if !self.notice.is_empty() {
            scene.ops.push(Op::Rect {
                x: 0.0,
                y: height as f32 - 26.0,
                width: width as f32,
                height: 26.0,
                style: color(35, 40, 48),
            });
            scene.ops.push(text(
                12.0,
                height as f32 - 23.0,
                &self.notice,
                color(230, 230, 235),
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
                style: color(35, 40, 48),
            });
            scene.ops.push(text(
                x + 12.0,
                y + 10.0,
                "Save attachment · local destination",
                color(235, 235, 240),
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
                color(90, 170, 240)
            } else {
                color(66, 74, 86)
            },
        });
        scene.ops.push(Op::Rect {
            x,
            y,
            width,
            height,
            style: color(27, 31, 38),
        });
        for (line, text_value) in label.lines().enumerate() {
            scene.ops.push(text(
                x + 7.0,
                y + 6.0 + line as f32 * 21.0,
                text_value,
                color(230, 232, 236),
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
                    style: color(230, 232, 236),
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
        if let Some((a, b)) = self.selection {
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            if index >= start.0 && index <= end.0 {
                let from = if index == start.0 {
                    start.1.min(plain.chars().count())
                } else {
                    0
                };
                let to = if index == end.0 {
                    end.1.min(plain.chars().count())
                } else {
                    plain.chars().count()
                };
                scene.ops.push(Op::Rect {
                    x: x + from as f32 * 8.4,
                    y,
                    width: to.saturating_sub(from) as f32 * 8.4,
                    height: 21.0,
                    style: color(55, 86, 120),
                });
            }
        }
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
        self.rows.push(TextRow { text: plain });
    }
    fn node(
        &mut self,
        node: &Node,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
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
                    style: color(45, 52, 61),
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
                    style: color(90, 170, 225),
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
                            color(45, 53, 63)
                        } else {
                            color(28, 33, 40)
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
                        self.node(child, x + 25.0, y, (width - 25.0).max(10.0), theme, scene);
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
            for child in &node.children {
                self.node(child, x, y, width, theme, scene);
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
fn color(r: u8, g: u8, b: u8) -> Style {
    Style::fg(Color::Rgb(r, g, b))
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
    use super::*;
    use misa_proto::view::{Action, Field, Span};
    use misa_value::Value;
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
        assert!(scene.ops.iter().any(|op| matches!(op, Op::Image { .. })));
        assert!(
            scene
                .ops
                .iter()
                .any(|op| matches!(op,Op::Rect {width,height,..} if *width==80.0 && *height==10.0))
        );
        assert!(app.rows.len() > 6, "long table cell must wrap");
        let raster = crate::paint::raster(&scene, Color::Rgb(20, 22, 26)).unwrap();
        assert!(raster.pixels().any(|pixel| pixel.0 == [255, 0, 0, 255]));
    }
}
