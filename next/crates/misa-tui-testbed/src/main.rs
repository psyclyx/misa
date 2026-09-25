//! Standalone, local-only document fixture runner. No Workspace or session is constructed.
//! Run with: cargo run -p misa-tui-testbed
//! n/Right/Tab and p/Left cycle scenes; j/k scroll; t changes theme; Ctrl-Q quits.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use misa_proto::view::{Action, ActionOn, Field, FieldKind, Kind, Node, Span, State};
use misa_tui_ui::{
    KeyOut, Screen,
    offline::{self, Control, Controller, Update},
};
use std::io::{self, IsTerminal};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    Semantic,
    Structured,
    Form,
}
impl Scene {
    const ALL: [Self; 3] = [Self::Semantic, Self::Structured, Self::Form];
    fn label(self) -> &'static str {
        match self {
            Self::Semantic => "semantic transcript",
            Self::Structured => "semantic structures",
            Self::Form => "local document form",
        }
    }
}

fn screen(width: u16, height: u16) -> Screen {
    Screen::new(width, height)
}

fn fixture(scene: Scene) -> Node {
    match scene {
        Scene::Semantic => semantic_transcript(),
        Scene::Structured => semantic_structures(),
        Scene::Form => form_fixture(),
    }
}
fn semantic_transcript() -> Node {
    Node::section("session")
        .id("fixture.session")
        .child(
            Node::text("session.title", [Span::plain("Fixture conversation")]).id("fixture.title"),
        )
        .child(
            Node::section("transcript")
                .id("fixture.transcript")
                .child(
                    Node::text(
                        "message.user",
                        [Span::plain("How does a local fixture render?")],
                    )
                    .id("fixture.user"),
                )
                .child(
                    Node::text(
                        "message.assistant",
                        [Span::plain(
                            "Through the semantic line renderer and the real terminal painter.",
                        )],
                    )
                    .id("fixture.answer")
                    .state(State::Done),
                )
                .child(
                    Node::new(
                        "tool.result",
                        Kind::Code {
                            lang: Some("rust".into()),
                            text: "let connected = false;\nassert!(!connected);".into(),
                        },
                    )
                    .id("fixture.code"),
                ),
        )
}
fn semantic_structures() -> Node {
    Node::section("session")
        .id("fixture.structures")
        .child(
            Node::new(
                "heading",
                Kind::Heading {
                    level: 2,
                    spans: vec![Span::plain("Semantic structures")],
                },
            )
            .id("fixture.heading"),
        )
        .child(
            Node::new(
                "list",
                Kind::List {
                    ordered: false,
                    items: vec![
                        vec![
                            Node::text("text", [Span::plain("First nested item")])
                                .id("fixture.one"),
                        ],
                        vec![
                            Node::text("text", [Span::plain("Second nested item")])
                                .id("fixture.two"),
                        ],
                    ],
                    markers: vec![Some(true), Some(false)],
                },
            )
            .id("fixture.list"),
        )
        .child(
            Node::new(
                "status",
                Kind::Meter {
                    label: "Local progress".into(),
                    value: 2.0,
                    max: 3.0,
                },
            )
            .id("fixture.meter"),
        )
}

fn form_fixture() -> Node {
    Node::section("session").id("fixture.form").child(
        Node::section("panel")
            .id("fixture.panel")
            .label("Local document question")
            .child(
                Node::new(
                    "panel.input",
                    Kind::Fields {
                        fields: vec![Field {
                            id: "answer".into(),
                            label: "Answer".into(),
                            value: String::new(),
                            hint: None,
                            read_only: false,
                            secret: false,
                            kind: FieldKind::Inline,
                        }],
                    },
                )
                .id("fixture.input")
                .action(Action {
                    id: "panel.submit".into(),
                    on: ActionOn::Submit,
                    label: Some("Submit".into()),
                    args: misa_value::Value::Null,
                }),
            )
            .action(Action {
                id: "panel.close".into(),
                on: ActionOn::Click,
                label: Some("Cancel".into()),
                args: misa_value::Value::Null,
            }),
    )
}

#[derive(Default)]
struct Testbed {
    scene: usize,
    theme: usize,
}
impl Testbed {
    fn label(&self, screen: &mut Screen) {
        screen.location = format!(
            "Fixture {}/{} · {} · theme {}",
            self.scene + 1,
            Scene::ALL.len(),
            Scene::ALL[self.scene].label(),
            ["dark", "light", "plain"][self.theme]
        );
        screen.notice = Some("n/p or ←/→: scene  j/k: scroll  t: theme  Ctrl-Q: quit".into());
    }
}
impl Controller for Testbed {
    fn key(&mut self, screen: &mut Screen, view: &Node, key: &KeyEvent) -> Control {
        if key.modifiers != KeyModifiers::NONE && key.modifiers != KeyModifiers::SHIFT {
            return Control::Pass;
        }
        if Scene::ALL[self.scene] == Scene::Form
            && view.children.iter().any(|node| node.role == "panel")
        {
            return Control::Pass;
        }
        let changed = match key.code {
            KeyCode::Char('n') | KeyCode::Right | KeyCode::Tab => {
                self.scene = (self.scene + 1) % Scene::ALL.len();
                true
            }
            KeyCode::Char('p') | KeyCode::Left | KeyCode::BackTab => {
                self.scene = (self.scene + Scene::ALL.len() - 1) % Scene::ALL.len();
                true
            }
            KeyCode::Char('t') => {
                self.theme = (self.theme + 1) % 3;
                screen.theme = match self.theme {
                    1 => misa_render::Theme::light(),
                    2 => misa_render::Theme::plain(),
                    _ => misa_render::Theme::dark(),
                };
                self.label(screen);
                return Control::Consumed;
            }
            KeyCode::Char('q') | KeyCode::Esc => return Control::Quit,
            KeyCode::Char('j') | KeyCode::Down => {
                screen.key(misa_tui_ui::Key::ScrollPage(1));
                return Control::Consumed;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                screen.key(misa_tui_ui::Key::ScrollPage(-1));
                return Control::Consumed;
            }
            _ => false,
        };
        if changed {
            screen.scroll = 0;
            screen.follow = true;
            self.label(screen);
            Control::Update(Update::Reset(fixture(Scene::ALL[self.scene])))
        } else {
            Control::Pass
        }
    }
    fn out(&mut self, screen: &mut Screen, out: KeyOut) -> Control {
        if let KeyOut::Intent(misa_kit::intent::Intent::Action { action, fields, .. }) = out {
            if action == "panel.submit" || action == "panel.close" {
                let answer = fields
                    .iter()
                    .find(|field| field.id == "answer")
                    .map(|field| field.value.as_str())
                    .unwrap_or("");
                let message = if action == "panel.close" {
                    "Form cancelled".into()
                } else {
                    format!("Local form submitted: {answer}")
                };
                screen.panel = None;
                return Control::Update(Update::Reset(
                    Node::section("session")
                        .id("fixture.result")
                        .child(Node::text("notice", [Span::plain(message)]).id("fixture.answer")),
                ));
            }
        }
        Control::Pass
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "misa-tui-testbed: local fixtures only; n/p cycle, j/k scroll, t theme, Ctrl-Q quit (Esc cancels form)"
        );
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("testbed requires a terminal (try --help)".into());
    }
    let (width, height) = crossterm::terminal::size()?;
    let mut screen = screen(width, height);
    let mut testbed = Testbed::default();
    testbed.label(&mut screen);
    offline::run(&mut screen, fixture(Scene::Semantic), &mut testbed).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyEvent};
    use tokio::sync::mpsc;
    #[tokio::test]
    async fn semantic_scene_cycle_and_resize_use_retained_screen_loop() {
        let mut screen = screen(60, 12);
        let mut controller = Testbed::default();
        controller.label(&mut screen);
        let (sender, events) = mpsc::channel(8);
        let (_updates, incoming) = mpsc::channel(1);
        sender
            .send(Ok(Event::Key(KeyEvent::new(
                KeyCode::Char('n'),
                KeyModifiers::NONE,
            ))))
            .await
            .unwrap();
        sender.send(Ok(Event::Resize(40, 10))).await.unwrap();
        sender
            .send(Ok(Event::Key(KeyEvent::new(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
            ))))
            .await
            .unwrap();
        let mut bytes = Vec::new();
        offline::drive(
            &mut screen,
            fixture(Scene::Semantic),
            &mut controller,
            events,
            incoming,
            &mut bytes,
        )
        .await
        .unwrap();
        let paint = String::from_utf8(bytes).unwrap();
        assert!(paint.contains("Fixture conversation"), "{paint}");
        assert!(paint.contains("Semantic structures"), "{paint}");
        assert_eq!((screen.width, screen.height), (40, 10));
        assert_eq!(controller.scene, 1);
    }
    #[tokio::test]
    async fn document_form_accepts_local_input_without_a_client_parser() {
        let mut screen = screen(60, 12);
        let (sender, events) = mpsc::channel(8);
        let (_updates, incoming) = mpsc::channel(1);
        for code in [
            KeyCode::Char('n'),
            KeyCode::Char('o'),
            KeyCode::Enter,
            KeyCode::Char('q'),
        ] {
            sender
                .send(Ok(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))))
                .await
                .unwrap();
        }
        let mut bytes = Vec::new();
        let mut controller = Testbed { scene: 2, theme: 0 };
        offline::drive(
            &mut screen,
            fixture(Scene::Form),
            &mut controller,
            events,
            incoming,
            &mut bytes,
        )
        .await
        .unwrap();
        let paint = String::from_utf8(bytes).unwrap();
        assert!(paint.contains("Local document question"));
        assert!(paint.contains("Local form submitted: no"));
        assert!(screen.panel.is_none());
    }
}
