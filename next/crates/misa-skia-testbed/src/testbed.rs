//! Shared offline fixtures for the default headless GPU driver and optional window.
//! No connection, daemon discovery, clipboard, or preferences are initialized here.
pub mod headless;
#[cfg(feature = "native")]
pub mod window;

use crate::{Op, Scene};
#[cfg(test)]
use misa_pixel_document::ui::PULSE_PERIOD;
use misa_pixel_document::ui::{DocumentUi, Key};
use misa_pixel_testbed::Dashboard;
use misa_pixel_ui::TextMetrics;
use misa_proto::view::{Action, ActionOn, Field, FieldKind, Kind, Node, Span};
use misa_style::Style;
use misa_window_core::{Event, PointerPhase, Size};
use std::sync::Arc;
use std::time::Duration;
#[cfg(any(feature = "native", test))]
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Native,
    Semantic,
}

struct Fixtures {
    mode: Mode,
    native: Dashboard,
    metrics: Arc<dyn TextMetrics>,
    semantic: DocumentUi,
    deadline: Option<Duration>,
}

impl Fixtures {
    fn new() -> Result<Self, String> {
        let metrics = misa_skia_paint::text_metrics()
            .map_err(|error| format!("Cannot load Skia text metrics: {error}"))?;
        Ok(Self {
            mode: Mode::Native,
            native: Dashboard::default(),
            semantic: DocumentUi::new(semantic_fixture(), metrics.clone()),
            metrics,
            deadline: None,
        })
    }

    /// The native view is idle; only a painted semantic pulse schedules a wakeup.
    #[cfg(any(feature = "native", test))]
    fn pulse_deadline(&self, start: Instant) -> Option<Instant> {
        (self.mode == Mode::Semantic)
            .then_some(self.deadline)
            .flatten()
            .map(|elapsed| start + elapsed)
    }

    fn select(&mut self, number: &str) {
        self.deadline = None;
        self.mode = match number {
            "1" => Mode::Native,
            "2" => Mode::Semantic,
            _ => self.mode,
        };
    }

    #[cfg(test)]
    fn frame(&mut self, width: u32, height: u32) -> Scene {
        self.frame_at(width, height, None)
    }

    fn frame_at(&mut self, width: u32, height: u32, elapsed: Option<Duration>) -> Scene {
        let mut scene = match self.mode {
            Mode::Native => {
                self.deadline = None;
                self.native.frame(width, height, self.metrics.as_ref())
            }
            Mode::Semantic => match elapsed {
                Some(elapsed) => {
                    let output = self
                        .semantic
                        .drive(Event::Redraw(Size { width, height }), elapsed);
                    self.deadline = output.deadline;
                    output.frame.expect("nonempty fixture")
                }
                None => {
                    self.deadline = None;
                    self.semantic.frame_at(width, height, Duration::ZERO)
                }
            },
        };
        // Shared overlay belongs to the local testbed, not the semantic fixture.
        scene.ops.push(Op::Rect {
            x: 0.0,
            y: (height as f32 - 26.0).max(0.0),
            width: width as f32,
            height: 26.0,
            style: Style::rgb(35, 40, 48),
        });
        scene.ops.push(Op::Text {
            x: 12.0,
            y: (height as f32 - 23.0).max(0.0),
            size: 14.0,
            style: Style::rgb(230, 232, 236),
            text: format!("Ctrl+1 native  ·  Ctrl+2 semantic  |  {:?}", self.mode),
        });
        scene
    }

    fn input(&mut self, event: Event, elapsed: Duration, width: u32) {
        match event {
            Event::Key(key) => self.key_at(key, elapsed),
            Event::Text(text) => {
                if self.mode == Mode::Native && self.native.menu_open() {
                    return;
                }
                if self.mode == Mode::Native {
                    if self.native.note_focused() {
                        self.native.insert_note(&text);
                    } else if text == " " {
                        self.native.toggle();
                    }
                } else {
                    self.semantic_input(Event::Text(text), elapsed);
                }
            }
            Event::Pointer { x, y, phase } => {
                if self.mode == Mode::Native {
                    // The native fixture models clicks; a release without a
                    // drag away is the click.
                    if phase == PointerPhase::Release {
                        self.native.click(x, y, width, self.metrics.as_ref());
                    }
                } else {
                    self.semantic_input(Event::Pointer { x, y, phase }, elapsed);
                }
            }
            Event::ContextMenu { x, y } if self.mode == Mode::Native => {
                self.native.context_menu(x, y)
            }
            Event::Wheel { delta } if self.mode == Mode::Native => self.native.scroll(delta),
            Event::ContextMenu { .. }
            | Event::Wheel { .. }
            | Event::Resize(_)
            | Event::Theme { .. }
            | Event::Redraw(_) => {
                if self.mode == Mode::Semantic {
                    self.semantic_input(event, elapsed);
                }
            }
        }
    }

    #[cfg(test)]
    fn key(&mut self, key: Key) {
        self.key_at(key, Duration::ZERO);
    }
    fn key_at(&mut self, key: Key, elapsed: Duration) {
        match self.mode {
            Mode::Native => {
                let menu_key = match key {
                    Key::Escape => Some(misa_pixel_ui::MenuKey::Escape),
                    Key::Up => Some(misa_pixel_ui::MenuKey::Up),
                    Key::Down => Some(misa_pixel_ui::MenuKey::Down),
                    Key::Home => Some(misa_pixel_ui::MenuKey::Home),
                    Key::End => Some(misa_pixel_ui::MenuKey::End),
                    Key::Enter { .. } => Some(misa_pixel_ui::MenuKey::Enter),
                    _ => None,
                };
                if menu_key.is_some_and(|k| self.native.menu_key(k, 500, self.metrics.as_ref())) {
                    return;
                }
                if self.native.menu_open() && key != Key::Menu {
                    return;
                }
                match key {
                    Key::Menu => {
                        self.native.context_menu(40.0, 150.0);
                        self.native.menu_key(
                            misa_pixel_ui::MenuKey::Home,
                            500,
                            self.metrics.as_ref(),
                        );
                    }
                    Key::Backspace => self.native.backspace_note(),
                    Key::Enter { newline: true } if self.native.note_focused() => {
                        self.native.insert_note("\n")
                    }
                    Key::Enter { .. } if !self.native.note_focused() => self.native.toggle(),
                    _ => {}
                }
            }
            Mode::Semantic => self.semantic_input(Event::Key(key), elapsed),
        }
    }

    fn semantic_input(&mut self, event: Event, elapsed: Duration) {
        if !self.semantic.drive(event, elapsed).commands.is_empty() {
            // Commands are intentionally never dispatched to a client or transport.
            self.semantic.notice("Fixture action only (not sent)");
        }
    }
}

fn semantic_fixture() -> Node {
    Node::section("session").id("session").children([
        Node::new(
            "markdown.heading",
            Kind::Heading {
                level: 1,
                spans: vec![Span::plain("Semantic fixture")],
            },
        )
        .id("heading"),
        Node::section("status.indicators")
            .id("fixture.status")
            .child(
                Node::new(
                    "indicator.activity",
                    Kind::Status {
                        text: "working".into(),
                    },
                )
                .id("fixture.activity"),
            ),
        Node::section("message.user")
            .id("message")
            .child(Node::text(
                "message.user",
                [
                    Span::plain("A local tree rendered by the semantic document UI."),
                    Span::code(" No daemon required."),
                ],
            )),
        Node::new(
            "panel.input",
            Kind::Fields {
                fields: vec![Field {
                    id: "note".into(),
                    label: "Try editing this field".into(),
                    value: "Local draft".into(),
                    hint: None,
                    kind: FieldKind::Inline,
                    read_only: false,
                    secret: false,
                }],
            },
        )
        .id("panel.input")
        .action(Action {
            id: "fixture.submit".into(),
            on: ActionOn::Submit,
            label: Some("Submit locally".into()),
            args: misa_value::Value::Null,
        }),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn has_text(ops: &[Op], needle: &str) -> bool {
        ops.iter().any(|op| match op {
            Op::Text { text, .. } => text.contains(needle),
            Op::Group { ops, .. } | Op::ClipRect { ops, .. } => has_text(ops, needle),
            _ => false,
        })
    }

    fn scene_text(ops: &[Op]) -> String {
        let mut text = String::new();
        for op in ops {
            match op {
                Op::Text { text: run, .. } => text.push_str(run),
                Op::Group { ops, .. } | Op::ClipRect { ops, .. } => text.push_str(&scene_text(ops)),
                _ => {}
            }
        }
        text
    }

    #[test]
    fn block_draft_wraps_without_changing_its_text_on_resize_or_reset() {
        let mut fixture = semantic_fixture();
        let Kind::Fields { fields } = &mut fixture.children.last_mut().unwrap().kind else {
            panic!("fixture field")
        };
        fields[0].kind = FieldKind::Block;
        let metrics = misa_skia_paint::text_metrics().unwrap();
        let mut app = DocumentUi::new(fixture.clone(), metrics);
        app.focus_control(Some(misa_pixel_document::ui::Control::Field {
            node: "panel.input".into(),
            field: "note".into(),
        }));
        let draft = " abcdefghijklmnopqrstuvwxyz\nlast";
        app.drive(Event::Text(draft.into()), Duration::ZERO);
        let narrow = app.frame(130, 440);
        let painted = scene_text(&narrow.ops);
        assert!(
            painted.contains("last"),
            "caret follows the final visual row"
        );
        assert!(
            !painted.contains("Local draft abcdefghijklmnopqrstuvwxyz"),
            "block uses soft rows"
        );
        assert_eq!(
            app.field_text("panel.input", "note"),
            Some(format!("Local draft{draft}").as_str())
        );
        app.frame(500, 440);
        app.set_view(fixture);
        assert_eq!(
            app.field_text("panel.input", "note"),
            Some(format!("Local draft{draft}").as_str())
        );
    }

    #[test]
    fn fake_clock_pulses_only_the_semantic_fixture() {
        let mut fixtures = Fixtures::new().expect("Skia text metrics for fixtures");
        let start = Instant::now();
        fixtures.frame_at(500, 320, Some(Duration::ZERO));
        assert_eq!(fixtures.pulse_deadline(start), None);

        fixtures.select("2");
        assert_eq!(fixtures.pulse_deadline(start), None);
        let first = fixtures.frame_at(500, 320, Some(Duration::ZERO));
        assert!(fixtures.semantic.animating());
        assert_eq!(fixtures.pulse_deadline(start), Some(start + PULSE_PERIOD));
        let same = fixtures.frame_at(500, 320, Some(Duration::from_millis(159)));
        assert_eq!(scene_text(&first.ops), scene_text(&same.ops));
        let changed = fixtures.frame_at(500, 320, Some(Duration::from_millis(320)));
        assert_ne!(scene_text(&first.ops), scene_text(&changed.ops));
        assert_eq!(
            fixtures.pulse_deadline(start),
            Some(start + Duration::from_millis(480))
        );

        fixtures.select("1");
        let native = fixtures.frame_at(500, 320, Some(Duration::from_secs(10)));
        assert!(has_text(&native.ops, "Native dashboard"));
        assert_eq!(fixtures.pulse_deadline(start), None);
    }

    #[test]
    fn selection_resize_and_keys_change_the_painted_fixture() {
        let mut fixtures = Fixtures::new().expect("Skia text metrics for fixtures");
        let native = fixtures.frame(500, 320);
        assert!(has_text(&native.ops, "Native dashboard"));
        fixtures.input(Event::Text(" ".into()), Duration::ZERO, 500);
        assert!(has_text(&fixtures.frame(500, 320).ops, "Selected"));
        fixtures.key(Key::Enter { newline: false });
        assert!(!has_text(&fixtures.frame(500, 320).ops, "Selected"));
        fixtures.key(Key::Enter { newline: false });
        assert!(has_text(&fixtures.frame(500, 320).ops, "Selected"));
        fixtures.select("2");
        let semantic = fixtures.frame(340, 480);
        assert_eq!((semantic.width, semantic.height), (340.0, 480.0));
        assert!(has_text(&semantic.ops, "Semantic fixture"));
        fixtures
            .semantic
            .focus_control(Some(misa_pixel_document::ui::Control::Field {
                node: "panel.input".into(),
                field: "note".into(),
            }));
        fixtures.input(Event::Text("!".into()), Duration::ZERO, 340);
        assert_eq!(
            fixtures.semantic.field_text("panel.input", "note"),
            Some("Local draft!")
        );
        assert!(has_text(&fixtures.frame(500, 320).ops, "Local draft!"));
        fixtures.key(Key::Enter { newline: false });
        assert_eq!(
            fixtures.semantic.notice_text(),
            "Fixture action only (not sent)"
        );
        fixtures.select("1");
        assert!(has_text(&fixtures.frame(500, 320).ops, "Selected"));
    }
}
