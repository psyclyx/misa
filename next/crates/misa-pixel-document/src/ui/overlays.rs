//! Misa-local modal controls and notices. No session effects run here: decisions are
//! returned to the coordinator as commands or focus/draft requests.
use super::{Command, Control, FONT_SIZE, FieldViewport, Key};
use misa_kit::editor::{Editor, Motion};
use misa_kit::intent;
use misa_kit::picker::{Accept, Picker};
use misa_pixel_ui::{TextFlow, TextMetrics};
use misa_proto::view::Choice;
use misa_value::Value;

#[derive(Default)]
pub(super) struct LocalOverlays {
    save: Option<SavePath>,
    picker: Option<Picker>,
    report: Option<Report>,
    notice: String,
    commands: Vec<intent::Command>,
}

struct SavePath {
    node: String,
    editor: Editor,
    viewport: FieldViewport,
    replace_selection: bool,
}

pub(super) enum OverlayAction {
    InsertCommand(String),
}

pub(super) enum Decision {
    Continue,
    Consumed {
        command: Option<Command>,
        focus: Option<Option<Control>>,
        action: Option<OverlayAction>,
    },
}
impl Decision {
    fn consumed() -> Self {
        Self::Consumed {
            command: None,
            focus: None,
            action: None,
        }
    }
}

pub(super) struct Report {
    pub title: String,
    entries: Vec<String>,
    pub offset: usize,
    width: f32,
    pub lines: Vec<String>,
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
    pub fn reflow(
        &mut self,
        width: f32,
        height: f32,
        metrics: &dyn TextMetrics,
        style: misa_style::Style,
    ) -> usize {
        let budget = (width - 96.0).max(1.0);
        let flow = TextFlow::new(metrics, FONT_SIZE);
        if self.width != budget {
            self.lines = self
                .entries
                .iter()
                .flat_map(|entry| {
                    flow.wrap(vec![(style, entry.clone())], budget)
                        .into_iter()
                        .map(|line| line.into_iter().map(|(_, text)| text).collect::<String>())
                })
                .collect();
            self.width = budget;
        }
        let spacing = flow.line_height() + 3.0;
        let visible = ((height - 140.0).max(spacing) / spacing).floor() as usize;
        self.offset = self.offset.min(self.lines.len().saturating_sub(visible));
        visible
    }
}

impl LocalOverlays {
    pub fn notice(&mut self, text: &str) {
        self.notice = text.into();
    }
    pub fn notice_text(&self) -> &str {
        &self.notice
    }
    pub fn declare_commands(&mut self, commands: Vec<intent::Command>) {
        self.commands = commands;
    }
    pub fn parse(&mut self, text: &str) -> Option<intent::Intent> {
        let parsed = intent::parse(text, &self.commands);
        let result = intent::intent(&parsed);
        if result.is_none() {
            self.notice(&format!("Cannot submit: {parsed:?}"));
        }
        result
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
    pub fn report_mut(&mut self) -> Option<&mut Report> {
        self.report.as_mut()
    }
    pub fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }
    pub fn save_text(&self) -> Option<&str> {
        self.save.as_ref().map(|save| save.editor.text())
    }
    pub fn save_cursor_viewport(&self) -> Option<(usize, FieldViewport)> {
        self.save
            .as_ref()
            .map(|save| (save.editor.cursor(), save.viewport))
    }
    pub fn set_save_viewport(&mut self, viewport: FieldViewport) {
        if let Some(save) = &mut self.save {
            save.viewport = viewport;
        }
    }
    pub fn saving(&self) -> bool {
        self.save.is_some()
    }
    pub fn pointer_blocked(&self) -> bool {
        self.picker.is_some() || self.report.is_some()
    }
    pub fn scroll(&mut self, delta: f32) -> bool {
        if let Some(report) = &mut self.report {
            report.offset = report
                .offset
                .saturating_add_signed((delta / 24.0).round() as isize);
            true
        } else {
            false
        }
    }
    pub fn open_save(&mut self, node: String) -> Decision {
        self.save = Some(SavePath {
            node,
            editor: Editor::new(),
            viewport: FieldViewport::default(),
            replace_selection: false,
        });
        Decision::Consumed {
            command: None,
            focus: Some(Some(Control::SavePath)),
            action: None,
        }
    }
    pub fn activate(&mut self, control: &Control) -> Decision {
        match control {
            Control::SaveConfirm => self.confirm_save(),
            Control::SaveCancel => {
                self.save = None;
                Decision::Consumed {
                    command: None,
                    focus: Some(None),
                    action: None,
                }
            }
            _ => Decision::Continue,
        }
    }
    fn confirm_save(&mut self) -> Decision {
        let Some(save) = &self.save else {
            return Decision::consumed();
        };
        if save.editor.text().trim().is_empty() {
            self.notice("Enter a local destination path");
            return Decision::consumed();
        }
        let command = Command::Save {
            node: save.node.clone(),
            destination: save.editor.text().into(),
        };
        self.save = None;
        Decision::Consumed {
            command: Some(command),
            focus: Some(None),
            action: None,
        }
    }
    pub fn text(&mut self, text: &str, focus: Option<&Control>) -> Decision {
        if self.report.is_some() {
            return Decision::consumed();
        }
        if let Some(picker) = &mut self.picker {
            for character in text.chars() {
                picker.type_char(character);
            }
            return Decision::consumed();
        }
        if let Some(save) = &mut self.save {
            if focus == Some(&Control::SavePath) {
                if std::mem::take(&mut save.replace_selection) {
                    save.editor.set_text("");
                }
                save.editor.insert(text);
            }
            return Decision::consumed();
        }
        Decision::Continue
    }
    pub fn key(&mut self, key: Key, focus: Option<&Control>) -> Decision {
        if let Some(report) = &mut self.report {
            match key {
                Key::Escape | Key::Enter { .. } => self.report = None,
                Key::Up => report.offset = report.offset.saturating_sub(1),
                Key::Down => report.offset = report.offset.saturating_add(1),
                Key::Home => report.offset = 0,
                Key::Copy => {
                    return Decision::Consumed {
                        command: Some(Command::Copy(report.entries.join("\n"))),
                        focus: None,
                        action: None,
                    };
                }
                _ => {}
            }
            return Decision::consumed();
        }
        if matches!(key, Key::Commands) && self.save.is_none() {
            let mut picker = Picker::new("Commands", Accept::Run);
            picker.set_items(
                self.commands
                    .iter()
                    .map(|command| Choice {
                        value: command.id.clone(),
                        label: command.label.clone(),
                        detail: Some(command.description.clone()),
                        metadata: None,
                    })
                    .collect(),
                false,
            );
            self.picker = Some(picker);
            return Decision::consumed();
        }
        if let Some(picker) = &mut self.picker {
            match key {
                Key::Escape => self.picker = None,
                Key::Up | Key::Tab { backward: true } => picker.move_selection(-1),
                Key::Down | Key::Tab { backward: false } => picker.move_selection(1),
                Key::Backspace => {
                    picker.backspace();
                }
                Key::Enter { .. } => {
                    if let Some(candidate) = picker.selected().cloned() {
                        return Decision::Consumed {
                            command: None,
                            focus: None,
                            action: Some(OverlayAction::InsertCommand(candidate.value)),
                        };
                    }
                }
                _ => {}
            }
            return Decision::consumed();
        }
        if let Some(save) = &mut self.save {
            match key {
                Key::Escape => {
                    self.save = None;
                    return Decision::Consumed {
                        command: None,
                        focus: Some(None),
                        action: None,
                    };
                }
                Key::Enter { .. } => return self.confirm_save(),
                Key::Tab { .. } => return Decision::Continue,
                Key::Copy => {
                    return Decision::Consumed {
                        command: (!save.editor.text().is_empty())
                            .then(|| Command::Copy(save.editor.text().into())),
                        focus: None,
                        action: None,
                    };
                }
                Key::SelectAll => save.replace_selection = true,
                _ if focus == Some(&Control::SavePath) => {
                    let replace = std::mem::take(&mut save.replace_selection);
                    match key {
                        Key::Backspace => {
                            if replace {
                                save.editor.set_text("");
                            } else {
                                save.editor.backspace();
                            }
                        }
                        Key::Delete => {
                            save.editor.delete();
                        }
                        Key::Left => {
                            save.editor.move_cursor(Motion::Left);
                        }
                        Key::Right => {
                            save.editor.move_cursor(Motion::Right);
                        }
                        Key::Home => {
                            save.editor.move_cursor(Motion::LineStart);
                        }
                        Key::End => {
                            save.editor.move_cursor(Motion::LineEnd);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            return Decision::consumed();
        }
        Decision::Continue
    }
    pub fn inserted(&mut self, success: bool) {
        if success {
            self.picker = None;
            self.notice("Command inserted · add arguments, then Enter to send");
        } else {
            self.notice("This view has no prompt field");
        }
    }
}
