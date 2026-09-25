//! Terminal-local drafts and decisions for semantic session panels.
use crate::prefs::DialogSettings;
use crate::{Key, KeyOut};
use misa_kit::intent::Intent;
use misa_proto::view::{ActionOn, Field, Kind, Node};

/// What somebody has typed into a panel.
///
/// A panel is the session's state — what question is open, what field it has — and this is the
/// client's: the text in the field before it is submitted, which belongs to whoever is typing
/// and is never sent until they say so.
#[derive(Clone, Debug, Default, PartialEq)]
struct PanelInput {
    /// The id of the panel node this is an answer to.
    panel: String,
    /// The field being typed into, by id. Empty when the panel has nothing to type into.
    field: String,
    text: String,
}

/// State parked with a session; drafts never cross scope boundaries.
#[derive(Default)]
pub struct PanelState {
    input: Option<PanelInput>,
}
#[derive(Default)]
pub struct PanelController {
    state: PanelState,
    notice: Option<Option<String>>,
}
pub struct Decision {
    pub out: Option<KeyOut>,
    /// None leaves the notice unchanged; Some(None) clears it.
    pub notice: Option<Option<String>>,
}
impl Decision {
    fn pass() -> Self {
        Self {
            out: None,
            notice: None,
        }
    }
}
impl PanelController {
    pub fn active(&self) -> bool {
        self.state.input.is_some()
    }
    pub fn clear(&mut self) {
        self.state = PanelState::default();
        self.notice = None;
    }
    pub fn park(&mut self) -> PanelState {
        self.notice = None;
        std::mem::take(&mut self.state)
    }
    pub fn restore(&mut self, state: PanelState) {
        self.state = state;
        self.notice = None;
    }
    fn finish(&mut self, out: KeyOut) -> Decision {
        Decision {
            out: Some(out),
            notice: self.notice.take(),
        }
    }
    /// Project the active draft onto a field without sending it to the session.
    pub fn resolve(&self, node: &mut Node) {
        if let (Kind::Fields { fields }, Some(state)) = (&mut node.kind, &self.state.input)
            && let Some(field) = fields.iter_mut().find(|field| field.id == state.field)
        {
            field.value = state.text.clone();
        }
    }
    /// A key that concerns an open panel, if one is open.
    ///
    /// The panel is modal in the terminal, the way the picker is: it is the one thing on the
    /// screen that is a question rather than something to read, so while it is up every key
    /// that is not the way out belongs to it. The two exceptions are the ways out of the
    /// *program*, because a person may always stop.
    ///
    /// A client that wanted a non-modal panel would put the field somewhere else; what it may
    /// not do is decide the panel is not a question.
    pub fn key(&mut self, panel: &Node, key: &Key, settings: &DialogSettings) -> Decision {
        if matches!(key, Key::Quit | Key::Interrupt) {
            return Decision::pass();
        }
        let asking = panel.id.clone();
        if self.state.input.as_ref().map(|state| &state.panel) != Some(&asking) {
            // A draft belongs to the question that asked for it: a value carried into the next
            // panel is a value nobody wrote.
            let field = panel_field(panel)
                .map(|field| field.id.clone())
                .unwrap_or_default();
            // The panel's actions are the source of truth for its affordances. The surface
            // renders them as buttons; putting a second, client-authored prose hint in the
            // status/notice lane makes the interaction disagree with the action model.
            self.notice = Some(None);
            self.state.input = Some(PanelInput {
                panel: asking,
                field,
                text: String::new(),
            });
        }
        if let Some(action) = panel
            .actions
            .iter()
            .find(|action| action.on == ActionOn::Click && settings.matches(&action.id, key))
        {
            self.state.input = None;
            self.notice = Some(None);
            return self.finish(KeyOut::Intent(Intent::Action {
                node: panel.id.clone(),
                action: action.id.clone(),
                args: action.args.clone(),
                fields: Vec::new(),
            }));
        }
        if panel
            .children
            .iter()
            .flat_map(|child| child.actions.iter())
            .any(|action| action.on == ActionOn::Submit && settings.matches(&action.id, key))
        {
            let out = self.submit_panel(panel);
            return self.finish(out);
        }
        let out = match key {
            Key::Eof
                if self
                    .state
                    .input
                    .as_ref()
                    .is_some_and(|panel| panel.text.is_empty()) =>
            {
                KeyOut::Quit
            }
            Key::Escape => self.dismiss_panel(panel, settings),
            Key::Submit if settings.matches("panel.submit", key) => self.submit_panel(panel),
            Key::Backspace | Key::Delete => {
                if let Some(state) = self.state.input.as_mut() {
                    state.text.pop();
                }
                KeyOut::Local
            }
            Key::Char(character) => {
                if let Some(state) = self.state.input.as_mut() {
                    state.text.push(*character);
                }
                KeyOut::Local
            }
            _ => KeyOut::Local,
        };
        self.finish(out)
    }

    /// Take the panel away, by the action the session offered for it.
    fn dismiss_panel(&mut self, panel: &Node, settings: &DialogSettings) -> KeyOut {
        let close = panel.actions.iter().find(|action| {
            action.on == ActionOn::Click
                && settings.key(&action.id).is_some_and(|key| key == "escape")
        });
        let Some(close) = close else {
            // A panel nobody can dismiss is the session's decision; this client will not
            // invent one, and it says so rather than eating the key in silence.
            self.notice = Some(Some("this panel has no way out".to_string()));
            return KeyOut::Local;
        };
        self.state.input = None;
        self.notice = Some(None);
        KeyOut::Intent(Intent::Action {
            node: panel.id.clone(),
            action: close.id.clone(),
            args: misa_value::Value::Null,
            fields: Vec::new(),
        })
    }

    /// Send what is in the field, if the panel asked for something.
    fn submit_panel(&mut self, panel: &Node) -> KeyOut {
        let Some(form) = panel
            .children
            .iter()
            .find(|child| matches!(&child.kind, Kind::Fields { fields } if !fields.is_empty()))
        else {
            return KeyOut::Local;
        };
        let Some(action) = form
            .actions
            .iter()
            .find(|action| action.on == ActionOn::Submit)
        else {
            return KeyOut::Local;
        };
        let Kind::Fields { fields: declared } = &form.kind else {
            return KeyOut::Local;
        };
        let typed = self
            .state
            .input
            .as_ref()
            .map(|state| state.text.clone())
            .unwrap_or_default();
        let focused = self
            .state
            .input
            .as_ref()
            .map(|state| state.field.clone())
            .unwrap_or_default();
        let fields = declared
            .iter()
            .map(|field| Field {
                value: if field.id == focused {
                    typed.clone()
                } else {
                    field.value.clone()
                },
                ..field.clone()
            })
            .collect::<Vec<_>>();
        // What was typed goes out of this client's hands as it leaves the screen: a secret
        // that stays in a field after it has been sent is a secret on a screen.
        if let Some(state) = self.state.input.as_mut() {
            state.text.clear();
        }
        self.notice = Some(None);
        KeyOut::Intent(Intent::Action {
            node: form.id.clone(),
            action: action.id.clone(),
            args: misa_value::Value::Null,
            fields,
        })
    }
}

/// The panel in a view, if the session has one open.
///
/// By role rather than by id: the id is the session's name for the panel — `login`,
/// `authorize` — and that is what an action has to name, so the role is what says what a node
/// *is*.
pub fn panel_of(view: &Node) -> Option<&Node> {
    if view.role == "panel" {
        return Some(view);
    }
    view.children.iter().find_map(panel_of)
}

/// The field a panel wants typed into, if it wants one.
fn panel_field(panel: &Node) -> Option<&Field> {
    panel.children.iter().find_map(|child| match &child.kind {
        Kind::Fields { fields }
            if child
                .actions
                .iter()
                .any(|action| action.on == ActionOn::Submit) =>
        {
            fields.iter().find(|field| !field.read_only)
        }
        _ => None,
    })
}
