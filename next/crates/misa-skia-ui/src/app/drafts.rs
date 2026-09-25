//! Local editable fields. The document supplies field definitions; edits and field scroll
//! positions belong to the client and survive replacement of the same field identity.
use super::{FieldViewport, document::DocumentStore};
use misa_kit::editor::Editor;
use misa_proto::sync::ViewOp;
use misa_proto::view::{Field, FieldKind, Kind, Node};
use std::collections::{BTreeMap, BTreeSet};

type Key = (String, String);

#[derive(Default)]
pub(super) struct Drafts {
    editors: BTreeMap<Key, Editor>,
    viewports: BTreeMap<Key, FieldViewport>,
    pending_select_all: Option<Key>,
}

impl Drafts {
    fn collect(node: &Node, fields: &mut Vec<(String, Field)>) {
        if let Kind::Fields { fields: values } = &node.kind {
            fields.extend(
                values
                    .iter()
                    .filter(|field| !field.read_only)
                    .map(|field| (node.id.clone(), field.clone())),
            );
        }
        for child in &node.children {
            Self::collect(child, fields);
        }
        if let Kind::List { items, .. } = &node.kind {
            for child in items.iter().flatten() {
                Self::collect(child, fields);
            }
        }
    }

    fn add(&mut self, node: &str, field: &Field) {
        self.editors
            .entry((node.into(), field.id.clone()))
            .or_insert_with(|| {
                let mut editor = Editor::new();
                let value = match &field.kind {
                    FieldKind::Choice {
                        selected: Some(value),
                        ..
                    } => value,
                    _ => &field.value,
                };
                editor.set_text(value);
                editor
            });
    }

    fn retain(&mut self, valid: &BTreeSet<Key>) {
        self.editors.retain(|key, _| valid.contains(key));
        self.viewports.retain(|key, _| valid.contains(key));
        self.pending_select_all = None;
    }

    /// Reset reconciles the complete addressed tree, including fields inside list items.
    /// Return the ordered editable fields for the caller's focus policy.
    pub fn reset(&mut self, tree: &Node) -> Vec<Key> {
        let mut fields = Vec::new();
        Self::collect(tree, &mut fields);
        let valid = fields
            .iter()
            .map(|(node, field)| (node.clone(), field.id.clone()))
            .collect();
        self.retain(&valid);
        for (node, field) in &fields {
            self.add(node, field);
        }
        fields
            .into_iter()
            .map(|(node, field)| (node, field.id))
            .collect()
    }

    /// Only changed subtrees can introduce defaults. The canonical document decides
    /// which identities are still live after the transaction has been applied.
    pub fn changed(&mut self, ops: &[ViewOp], document: &DocumentStore) -> Option<Key> {
        let mut fields = Vec::new();
        for op in ops {
            if let ViewOp::Insert { node, .. } | ViewOp::Replace { node, .. } = op {
                Self::collect(node, &mut fields);
            }
        }
        let panel_input = fields
            .iter()
            .find(|(node, _)| node == "panel.input")
            .map(|(node, field)| (node.clone(), field.id.clone()));
        for (node, field) in fields {
            self.add(&node, &field);
        }
        self.editors.retain(|(id, field), _| {
            document
                .field(id, field)
                .is_some_and(|value| !value.read_only)
        });
        self.viewports
            .retain(|key, _| self.editors.contains_key(key));
        if self
            .pending_select_all
            .as_ref()
            .is_some_and(|key| !self.editors.contains_key(key))
        {
            self.pending_select_all = None;
        }
        panel_input
    }

    pub fn select_all(&mut self, node: &str, field: &str) {
        let key = (node.into(), field.into());
        self.pending_select_all = self.editors.contains_key(&key).then_some(key);
    }
    pub fn clear_selection(&mut self) {
        self.pending_select_all = None;
    }
    pub fn focus_changed(&mut self, focus: Option<&super::Control>) {
        if self.pending_select_all.as_ref().is_some_and(|(node, field)| {
            !matches!(focus, Some(super::Control::Field { node: focused_node, field: focused_field })
                if node == focused_node && field == focused_field)
        }) {
            self.clear_selection();
        }
    }
    fn take_selection(&mut self, node: &str, field: &str) -> bool {
        self.pending_select_all
            .take()
            .is_some_and(|key| key.0 == node && key.1 == field)
    }
    pub fn insert(&mut self, node: &str, field: &str, text: &str) {
        let replace = self.take_selection(node, field);
        if let Some(editor) = self.editor_mut(node, field) {
            if replace {
                editor.set_text("");
            }
            editor.insert(text);
        }
    }
    pub fn backspace(&mut self, node: &str, field: &str) {
        let replace = self.take_selection(node, field);
        if let Some(editor) = self.editor_mut(node, field) {
            if replace {
                editor.set_text("");
            } else {
                editor.backspace();
            }
        }
    }
    pub fn contains(&self, node: &str, field: &str) -> bool {
        self.editors.contains_key(&(node.into(), field.into()))
    }
    pub fn last(&self) -> Option<Key> {
        self.editors.keys().next_back().cloned()
    }
    fn prompt(&self) -> Option<Key> {
        self.editors
            .keys()
            .find(|(_, field)| field == "prompt")
            .cloned()
    }
    pub fn text(&self, node: &str, field: &str) -> Option<&str> {
        self.editors
            .get(&(node.into(), field.into()))
            .map(Editor::text)
    }
    pub fn editor_mut(&mut self, node: &str, field: &str) -> Option<&mut Editor> {
        self.editors.get_mut(&(node.into(), field.into()))
    }
    pub fn cursor(&self, node: &str, field: &str) -> Option<usize> {
        self.editors
            .get(&(node.into(), field.into()))
            .map(Editor::cursor)
    }
    pub fn viewport(&self, node: &str, field: &str) -> FieldViewport {
        self.viewports
            .get(&(node.into(), field.into()))
            .copied()
            .unwrap_or_default()
    }
    pub fn set_viewport(&mut self, node: &str, field: &str, viewport: FieldViewport) {
        if self.contains(node, field) {
            self.viewports.insert((node.into(), field.into()), viewport);
        }
    }
    pub fn clear_secrets(&mut self, document: &DocumentStore) -> Vec<String> {
        let mut dirty = Vec::new();
        for ((id, field), editor) in &mut self.editors {
            if document.field(id, field).is_some_and(|value| value.secret) {
                editor.set_text("");
                self.viewports.remove(&(id.clone(), field.clone()));
                dirty.push(id.clone());
            }
        }
        dirty
    }
    /// Recover only into an empty prompt; never overwrite more recent input.
    pub fn recover_prompt(&mut self, text: &str) -> Option<String> {
        let key = self.prompt()?;
        let editor = self.editors.get_mut(&key)?;
        if !editor.text().is_empty() {
            return None;
        }
        editor.set_text(text);
        Some(key.0)
    }
    pub fn insert_command(&mut self, value: &str) -> Option<Key> {
        let key = self.prompt()?;
        self.editors.get_mut(&key)?.set_text(&format!("/{value} "));
        Some(key)
    }
    pub fn discrete(&self, node: &str, field: &str, document: &DocumentStore) -> bool {
        document
            .field(node, field)
            .is_some_and(|value| matches!(value.kind, FieldKind::Bool | FieldKind::Choice { .. }))
    }
    pub fn cycle(&mut self, node: &str, field: &str, document: &DocumentStore) {
        self.clear_selection();
        let kind = document.field(node, field).map(|value| &value.kind);
        if let Some(editor) = self.editor_mut(node, field) {
            match kind {
                Some(FieldKind::Bool) => editor.set_text(if editor.text() == "true" {
                    "false"
                } else {
                    "true"
                }),
                Some(FieldKind::Choice { options, .. }) if !options.is_empty() => {
                    let next = options
                        .iter()
                        .position(|option| option.value == editor.text())
                        .map(|index| (index + 1) % options.len())
                        .unwrap_or(0);
                    editor.set_text(&options[next].value);
                }
                _ => {}
            }
        }
    }
    pub fn overlay(&self, node: &str, fields: &mut [Field]) {
        for field in fields {
            if let Some(value) = self.text(node, &field.id) {
                field.value = value.into();
            }
        }
    }
    pub fn submit_composer(&mut self, node: &str) {
        for ((id, _), editor) in &mut self.editors {
            if id == node {
                editor.submit();
            }
        }
    }
    #[cfg(test)]
    pub fn identity(&self, node: &str, field: &str) -> Option<*const Editor> {
        self.editors
            .get(&(node.into(), field.into()))
            .map(std::ptr::from_ref)
    }
}
