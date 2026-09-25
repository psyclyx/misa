//! Terminal-local scope state. Host dialogs are parked here; `ui.dialogs` is
//! only their projected render surface, rebuilt before each paint.
use crate::{ConnectedScreen, PanelState, dialogs::Dialogs, retained::Retained};
use misa_proto::{Node, view::BlobRef};
use std::collections::BTreeMap;

/// What a scope transition invalidates. Painting is independent of image discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    None,
    Redraw,
    Documents,
}
impl Change {
    pub fn images(self) -> bool {
        matches!(self, Self::Documents)
    }
    pub fn redraw(self) -> bool {
        !matches!(self, Self::None)
    }
}

struct Surface {
    dialogs: Dialogs,
    composer: misa_tui_app::ParkedInput,
    panel: PanelState,
    reader: misa_tui_app::Reader,
}

pub(super) struct ScopeState {
    id: String,
    pub retained: Retained,
    pub contributions: BTreeMap<String, Retained>,
    pub pending: Vec<BlobRef>,
    pub uploads: usize,
    pub generation: u64,
    surface: Option<Surface>,
}
impl ScopeState {
    fn new(id: String, generation: u64, screen: &ConnectedScreen) -> Self {
        Self {
            id,
            retained: Retained::new(Node::section("session").id("session"), &screen.ui),
            contributions: BTreeMap::new(),
            pending: Vec::new(),
            uploads: 0,
            generation,
            surface: None,
        }
    }
    fn park(&mut self, screen: &mut ConnectedScreen) {
        debug_assert!(self.surface.is_none());
        self.surface = Some(Surface {
            dialogs: std::mem::take(&mut screen.dialogs),
            composer: screen.ui.composer.park(),
            panel: screen.ui.park_panel(),
            reader: screen.ui.park_reader(),
        });
    }
    fn restore(&mut self, screen: &mut ConnectedScreen) {
        let surface = self
            .surface
            .take()
            .expect("only parked scopes can be restored");
        screen.dialogs = surface.dialogs;
        screen.ui.composer.restore(surface.composer);
        screen.ui.restore_panel(surface.panel);
        screen.ui.restore_reader(surface.reader);
    }
    fn image_blobs(&self) -> Vec<BlobRef> {
        self.retained
            .image_blobs()
            .into_iter()
            .chain(self.contributions.values().flat_map(|c| c.image_blobs()))
            .collect()
    }
}

pub(super) struct Scopes {
    pub active: ScopeState,
    parked: BTreeMap<String, ScopeState>,
    // Never reuse a generation, even after forgetting a scope with an upload in flight.
    next_generation: u64,
}
impl Scopes {
    pub fn new(screen: &ConnectedScreen) -> Self {
        Self {
            active: ScopeState::new(String::new(), 0, screen),
            parked: BTreeMap::new(),
            next_generation: 0,
        }
    }
    pub fn image_blobs(&self) -> Vec<BlobRef> {
        self.active.image_blobs()
    }
    pub fn activate(&mut self, next: String, screen: &mut ConnectedScreen) -> Change {
        if self.active.id == next {
            return Change::None;
        }
        if self.active.id.is_empty() {
            if let Some(mut parked) = self.parked.remove(&next) {
                parked.restore(screen);
                self.active = parked;
                screen.ui.activate_draft_scope(next);
                return Change::Documents;
            }
            screen.ui.enter_draft_scope(next.clone());
            self.active.id = next;
            return Change::Redraw;
        }
        screen.ui.remember_draft();
        self.active.park(screen);
        let old = std::mem::replace(
            &mut self.active,
            self.parked.remove(&next).unwrap_or_else(|| {
                self.next_generation = self.next_generation.wrapping_add(1);
                ScopeState::new(next.clone(), self.next_generation, screen)
            }),
        );
        self.parked.insert(old.id.clone(), old);
        if self.active.surface.is_some() {
            self.active.restore(screen);
            screen.ui.activate_draft_scope(next);
        } else {
            screen.ui.restore_draft_scope(next);
            screen.ui.reset_reader_viewport();
        }
        Change::Documents
    }
    pub fn forget(&mut self, id: &str, screen: &mut ConnectedScreen) -> Change {
        self.parked.remove(id);
        if self.active.id != id {
            return Change::None;
        }
        self.next_generation = self.next_generation.wrapping_add(1);
        // Forgetting an active scope discards its editor, host dialogs, reader,
        // transcript and attachments together. The projected dialog is repainted.
        screen.dialogs = Dialogs::default();
        screen.ui.composer.set_text("");
        screen.ui.composer.clear_picker();
        screen.ui.clear_panel();
        screen.ui.clear_selection();
        screen.ui.reset_reader_viewport();
        self.active = ScopeState::new(String::new(), self.next_generation, screen);
        Change::Documents
    }
    pub fn discard_attachments(&mut self) {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.active.generation = self.next_generation;
        self.active.pending.clear();
        self.active.uploads = 0;
    }
    /// An upload belongs to its originating scope, not the currently visible editor.
    pub fn uploaded(
        &mut self,
        generation: u64,
        result: Result<BlobRef, String>,
    ) -> Option<Result<(), String>> {
        let visible = self.active.generation == generation;
        let state = if visible {
            &mut self.active
        } else {
            self.parked
                .values_mut()
                .find(|state| state.generation == generation)?
        };
        state.uploads = state.uploads.saturating_sub(1);
        let outcome = result.map(|blob| {
            state.pending.push(blob);
        });
        visible.then_some(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Accept, Key, Picker};
    use misa_proto::view::{Action, ActionOn, Field, FieldKind, Kind, Span};

    fn panel() -> Node {
        Node::section("session").id("session").child(
            Node::section("panel").id("login").child(
                Node::new(
                    "fields",
                    Kind::Fields {
                        fields: vec![Field {
                            id: "token".into(),
                            label: "Token".into(),
                            value: String::new(),
                            hint: None,
                            read_only: false,
                            secret: true,
                            kind: FieldKind::Inline,
                        }],
                    },
                )
                .id("fields")
                .action(Action {
                    id: "submit".into(),
                    on: ActionOn::Submit,
                    label: None,
                    args: misa_value::Value::Null,
                }),
            ),
        )
    }

    #[test]
    fn switch_restores_open_picker_panel_selection_and_transcript() {
        let mut screen = ConnectedScreen::new(80, 24);
        let mut scopes = Scopes::new(&screen);
        scopes.activate("A".into(), &mut screen);
        let view = panel();
        scopes.active.retained = Retained::new(view.clone(), &screen.ui);
        screen
            .ui
            .composer
            .open_host_picker(Picker::new("choices", Accept::Run), ":");
        screen.ui.panel_key(&view, &Key::Char('s'));
        assert!(screen.ui.panel_active());
        screen.ui.composer.set_mode(misa_tui_app::ed::Mode::Normal);
        let text = Node::section("session")
            .id("session")
            .child(Node::text("line", [Span::plain("selected text")]));
        screen.ui.selection_key(&text, &Key::StartSelection);
        assert!(screen.ui.has_selection());
        let generation = scopes.active.generation;
        scopes.activate("B".into(), &mut screen);
        assert!(screen.ui.composer.picker().is_none());
        assert!(!screen.ui.panel_active());
        assert!(!screen.ui.has_selection());
        assert_ne!(scopes.active.generation, generation);
        scopes.activate("A".into(), &mut screen);
        assert_eq!(scopes.active.generation, generation);
        assert!(screen.ui.composer.has_picker());
        assert!(screen.ui.panel_active());
        assert!(screen.ui.has_selection());
        assert_eq!(scopes.active.retained.interaction(), view);
    }

    #[test]
    fn forget_drops_parked_state_and_ignores_late_uploads() {
        let mut screen = ConnectedScreen::new(80, 24);
        let mut scopes = Scopes::new(&screen);
        scopes.activate("A".into(), &mut screen);
        let old = scopes.active.generation;
        scopes.active.pending.push(BlobRef {
            hash: "old".into(),
            len: 1,
            media: None,
        });
        scopes.active.uploads = 1;
        scopes.active.retained = Retained::new(Node::section("old").id("old"), &screen.ui);
        scopes.activate("B".into(), &mut screen);
        assert_eq!(scopes.forget("A", &mut screen), Change::None);
        scopes.activate("A".into(), &mut screen);
        assert_ne!(scopes.active.generation, old);
        assert!(scopes.active.pending.is_empty());
        assert_eq!(scopes.active.uploads, 0);
        assert_eq!(scopes.active.retained.interaction().id, "session");
        assert!(
            scopes
                .uploaded(
                    old,
                    Ok(BlobRef {
                        hash: "late".into(),
                        len: 1,
                        media: None
                    })
                )
                .is_none()
        );
        assert!(scopes.active.pending.is_empty());
        let current = scopes.active.generation;
        assert_eq!(scopes.forget("A", &mut screen), Change::Documents);
        scopes.activate("A".into(), &mut screen);
        assert_ne!(scopes.active.generation, current);
        assert_eq!(scopes.active.retained.interaction().id, "session");
    }
}
