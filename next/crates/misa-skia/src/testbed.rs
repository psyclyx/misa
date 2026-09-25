//! Explicitly invoked, offline interactive Skia fixture window.
//! `misa-skia --testbed`: Ctrl+1 native view, Ctrl+2 semantic App view.
//! No connection, daemon discovery, clipboard, or preferences are initialized here.
mod native;

use crate::app::{App, Key};
use crate::{Op, Scene};
use misa_proto::view::{Action, ActionOn, Field, FieldKind, Kind, Node, Span};
use misa_render::{Color, Style};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Native,
    Semantic,
}

struct Fixtures {
    mode: Mode,
    native: native::Dashboard,
    semantic: App,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            mode: Mode::Native,
            native: native::Dashboard { selected: false },
            semantic: App::new(semantic_fixture()),
        }
    }

    fn select(&mut self, number: &str) {
        self.mode = match number {
            "1" => Mode::Native,
            "2" => Mode::Semantic,
            _ => self.mode,
        };
    }

    fn frame(&mut self, width: u32, height: u32) -> Scene {
        let mut scene = match self.mode {
            Mode::Native => self.native.frame(width, height),
            Mode::Semantic => self.semantic.frame(width, height),
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

    fn key(&mut self, key: Key) {
        match self.mode {
            Mode::Native => {
                if matches!(key, Key::Enter { .. })
                    || matches!(key, Key::Text(ref text) if text == " ")
                {
                    self.native.toggle();
                }
            }
            Mode::Semantic => {
                let commands = self.semantic.key(key);
                if !commands.is_empty() {
                    // Commands are intentionally never dispatched to a client or transport.
                    self.semantic.notice = "Fixture action only (not sent)".into();
                }
            }
        }
    }

    fn click(&mut self, x: f32, y: f32, width: u32) {
        match self.mode {
            Mode::Native => {
                self.native.click(x, y, width);
            }
            Mode::Semantic => {
                if !self.semantic.pointer(x, y, false).is_empty() {
                    self.semantic.notice = "Fixture action only (not sent)".into();
                }
            }
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
        Node::section("message.user")
            .id("message")
            .child(Node::text(
                "message.user",
                [
                    Span::plain("A local tree rendered via App::frame."),
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

pub fn run() -> Result<(), String> {
    let events = EventLoop::new().map_err(|error| error.to_string())?;
    let mut host = Host {
        fixtures: Fixtures::new(),
        window: None,
        surface: None,
        modifiers: ModifiersState::empty(),
        cursor: (0.0, 0.0),
        error: None,
    };
    events
        .run_app(&mut host)
        .map_err(|error| error.to_string())?;
    host.error.map_or(Ok(()), Err)
}

struct Host {
    fixtures: Fixtures,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    modifiers: ModifiersState,
    cursor: (f32, f32),
    error: Option<String>,
}

impl Host {
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn paint(&mut self) -> Result<(), String> {
        let size = self.window.as_ref().ok_or("No window")?.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let scene = self.fixtures.frame(size.width, size.height);
        let image = crate::paint::raster(&scene, Color::Rgb(20, 22, 26))?;
        crate::window::present_pixels(self.surface.as_mut().ok_or("No window surface")?, &image)
    }

    fn keyboard(&mut self, key: WinitKey) {
        let command = self.modifiers.control_key() || self.modifiers.super_key();
        match key {
            WinitKey::Character(ref text) if command && (text == "1" || text == "2") => {
                self.fixtures.select(text)
            }
            WinitKey::Named(NamedKey::Enter) => self.fixtures.key(Key::Enter {
                newline: self.modifiers.shift_key(),
            }),
            WinitKey::Named(NamedKey::Tab) => self.fixtures.key(Key::Tab {
                backward: self.modifiers.shift_key(),
            }),
            WinitKey::Named(NamedKey::Backspace) => self.fixtures.key(Key::Backspace),
            WinitKey::Named(NamedKey::Delete) => self.fixtures.key(Key::Delete),
            WinitKey::Named(NamedKey::ArrowLeft) => self.fixtures.key(Key::Left),
            WinitKey::Named(NamedKey::ArrowRight) => self.fixtures.key(Key::Right),
            WinitKey::Named(NamedKey::Home) => self.fixtures.key(Key::Home),
            WinitKey::Named(NamedKey::End) => self.fixtures.key(Key::End),
            WinitKey::Named(NamedKey::Escape) => self.fixtures.key(Key::Escape),
            WinitKey::Named(NamedKey::Space) if !command => {
                self.fixtures.key(Key::Text(" ".into()))
            }
            WinitKey::Character(text) if !command => self.fixtures.key(Key::Text(text.to_string())),
            _ => {}
        }
        self.redraw();
    }
}

impl ApplicationHandler for Host {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| {
            let window = Arc::new(
                events
                    .create_window(
                        Window::default_attributes()
                            .with_title(
                                "misa · local Skia testbed · Ctrl+1 native · Ctrl+2 semantic",
                            )
                            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 640.0)),
                    )
                    .map_err(|error| error.to_string())?,
            );
            window.set_ime_allowed(true);
            let context =
                softbuffer::Context::new(window.clone()).map_err(|error| error.to_string())?;
            self.surface = Some(
                softbuffer::Surface::new(&context, window.clone())
                    .map_err(|error| error.to_string())?,
            );
            self.window = Some(window);
            self.redraw();
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.error = Some(error);
            events.exit();
        }
    }

    fn window_event(&mut self, events: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => events.exit(),
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.paint() {
                    self.error = Some(error);
                    events.exit();
                }
            }
            WindowEvent::Resized(_) => self.redraw(),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32)
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let width = self
                    .window
                    .as_ref()
                    .map_or(0, |window| window.inner_size().width);
                self.fixtures.click(self.cursor.0, self.cursor.1, width);
                self.redraw();
            }
            WindowEvent::MouseWheel { delta, .. } if self.fixtures.mode == Mode::Semantic => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
                    MouseScrollDelta::PixelDelta(position) => -position.y as f32,
                };
                self.fixtures.semantic.scroll(delta);
                self.redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.fixtures.key(Key::Text(text));
                self.redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.keyboard(event.logical_key)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn has_text(ops: &[Op], needle: &str) -> bool {
        ops.iter().any(|op| match op {
            Op::Text { text, .. } => text.contains(needle),
            Op::Group { ops, .. } => has_text(ops, needle),
            _ => false,
        })
    }

    #[test]
    fn selection_resize_and_keys_change_the_painted_fixture() {
        let mut fixtures = Fixtures::new();
        let native = fixtures.frame(500, 320);
        assert!(has_text(&native.ops, "Native dashboard"));
        fixtures.key(Key::Enter { newline: false });
        assert!(has_text(&fixtures.frame(500, 320).ops, "Selected"));
        fixtures.select("2");
        let semantic = fixtures.frame(340, 280);
        assert_eq!((semantic.width, semantic.height), (340.0, 280.0));
        assert!(has_text(&semantic.ops, "Semantic fixture"));
        fixtures.semantic.focus = Some(crate::app::Control::Field {
            node: "panel.input".into(),
            field: "note".into(),
        });
        fixtures.key(Key::Text("!".into()));
        assert_eq!(
            fixtures.semantic.field_text("panel.input", "note"),
            Some("Local draft!")
        );
        assert!(has_text(&fixtures.frame(500, 320).ops, "Local draft!"));
        fixtures.key(Key::Enter { newline: false });
        assert_eq!(fixtures.semantic.notice, "Fixture action only (not sent)");
        fixtures.select("1");
        assert!(has_text(&fixtures.frame(500, 320).ops, "Selected"));
    }
}
