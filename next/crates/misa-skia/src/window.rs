//! A real desktop window consuming the same Skia scene used by PNG export.
use crate::app::{App, Command, Key};
use crate::connection::{self, Update};
use std::num::NonZeroU32;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

pub fn run(
    view: misa_proto::view::Node,
    ticket: Option<String>,
    snapshot: Option<String>,
) -> Result<(), String> {
    let events = EventLoop::<Update>::with_user_event()
        .build()
        .map_err(|error| error.to_string())?;
    let outgoing = ticket.map(|ticket| connection::start(ticket, events.create_proxy()));
    let mut host = Host {
        app: App::new(view),
        window: None,
        surface: None,
        modifiers: ModifiersState::empty(),
        cursor: (0.0, 0.0),
        dragging: false,
        clipboard: arboard::Clipboard::new().ok(),
        outgoing,
        snapshot,
        error: None,
    };
    events
        .run_app(&mut host)
        .map_err(|error| error.to_string())?;
    host.error.map_or(Ok(()), Err)
}
struct Host {
    app: App,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    modifiers: ModifiersState,
    cursor: (f32, f32),
    dragging: bool,
    clipboard: Option<arboard::Clipboard>,
    outgoing: Option<tokio::sync::mpsc::Sender<Command>>,
    snapshot: Option<String>,
    error: Option<String>,
}
impl Host {
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
    fn commands(&mut self, commands: Vec<Command>) {
        for command in commands {
            if let Command::Copy(text) = command {
                self.app.notice = match self
                    .clipboard
                    .as_mut()
                    .ok_or_else(|| "No clipboard is available".to_string())
                    .and_then(|clipboard| {
                        clipboard.set_text(text).map_err(|error| error.to_string())
                    }) {
                    Ok(()) => "Copied selection".into(),
                    Err(error) => error,
                };
            } else if let Some(outgoing) = &self.outgoing {
                if outgoing.try_send(command).is_err() {
                    self.app.notice = "The session is busy or disconnected".into();
                }
            } else {
                self.app.notice = "Connect to a session to use this action".into();
            }
        }
        self.redraw();
    }
    fn key(&mut self, key: Key) {
        let commands = self.app.key(key);
        self.commands(commands);
    }
    fn paint(&mut self) -> Result<(), String> {
        let Some(window) = &self.window else {
            return Ok(());
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let scene = self.app.frame(size.width, size.height);
        let image = crate::paint::raster(&scene, misa_render::Color::Rgb(20, 22, 26))?;
        if let Some(path) = &self.snapshot {
            image.save(path).map_err(|error| error.to_string())?;
        }
        let surface = self.surface.as_mut().ok_or("No window surface")?;
        surface
            .resize(
                NonZeroU32::new(size.width).unwrap(),
                NonZeroU32::new(size.height).unwrap(),
            )
            .map_err(|error| error.to_string())?;
        let mut buffer = surface.buffer_mut().map_err(|error| error.to_string())?;
        for (target, pixel) in buffer.iter_mut().zip(image.pixels()) {
            *target = ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | (pixel[2] as u32);
        }
        buffer.present().map_err(|error| error.to_string())
    }
}
impl ApplicationHandler<Update> for Host {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| {
            let window = Arc::new(
                events
                    .create_window(
                        Window::default_attributes()
                            .with_title("misa · pixels")
                            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 720.0)),
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
    fn user_event(&mut self, _: &ActiveEventLoop, update: Update) {
        match update {
            Update::View(view) => self.app.set_view(view),
            Update::Info(info) => self.app.info = Some(info),
            Update::Image { hash, image } => {
                self.app.images.insert(hash, image);
            }
            Update::Notice(notice) => self.app.notice = notice,
        }
        self.redraw();
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
                self.cursor = (position.x as f32, position.y as f32);
                if self.dragging {
                    let commands = self.app.pointer(self.cursor.0, self.cursor.1, true);
                    self.commands(commands);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
                if self.dragging {
                    let commands = self.app.pointer(self.cursor.0, self.cursor.1, false);
                    self.commands(commands);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
                    MouseScrollDelta::PixelDelta(position) => -position.y as f32,
                };
                self.app.scroll(delta);
                self.redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.key(Key::Text(text)),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command = self.modifiers.control_key() || self.modifiers.super_key();
                match event.logical_key {
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("c") => {
                        self.key(Key::Copy)
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("a") => {
                        self.key(Key::SelectAll)
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("v") => {
                        if let Some(clipboard) = &mut self.clipboard {
                            if let Ok(text) = clipboard.get_text() {
                                self.key(Key::Text(text));
                            }
                        }
                    }
                    WinitKey::Named(NamedKey::Enter) => self.key(Key::Enter {
                        newline: self.modifiers.shift_key(),
                    }),
                    WinitKey::Named(NamedKey::Tab) => self.key(Key::Tab {
                        backward: self.modifiers.shift_key(),
                    }),
                    WinitKey::Named(NamedKey::Escape) => self.key(Key::Escape),
                    WinitKey::Named(NamedKey::Backspace) => self.key(Key::Backspace),
                    WinitKey::Named(NamedKey::Delete) => self.key(Key::Delete),
                    WinitKey::Named(NamedKey::ArrowLeft) => self.key(Key::Left),
                    WinitKey::Named(NamedKey::ArrowRight) => self.key(Key::Right),
                    WinitKey::Named(NamedKey::Home) => self.key(Key::Home),
                    WinitKey::Named(NamedKey::End) => self.key(Key::End),
                    WinitKey::Character(text) if !command => self.key(Key::Text(text.to_string())),
                    WinitKey::Named(NamedKey::Space) if !command => self.key(Key::Text(" ".into())),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
