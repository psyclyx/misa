//! A real desktop window consuming the same Skia scene used by PNG export.
use crate::connection::{self, Update};
use crate::workspace::Action;
use misa_pixel_document::ui::{Command, Key};
mod session_views;
use misa_window_core::{Clock, Event, MonotonicClock, Size};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use session_views::{SessionViews, StateChange};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

pub fn run(
    view: misa_proto::view::Node,
    ticket: Option<String>,
    snapshot: Option<String>,
    live: bool,
) -> Result<(), String> {
    let events = EventLoop::<Update>::with_user_event()
        .build()
        .map_err(|error| error.to_string())?;
    let show_chooser = live && ticket.is_none();
    let outgoing = live.then(|| connection::start(ticket, events.create_proxy()));
    let metrics = misa_skia_paint::text_metrics()
        .map_err(|error| format!("Cannot load Skia text metrics: {error}"))?;
    let mut host = Host {
        appearance: crate::preferences::appearance(),
        appearance_writer: crate::preferences::appearance_writer(events.create_proxy()),
        views: SessionViews::new(view, metrics, show_chooser),
        window: None,
        surface: None,
        modifiers: ModifiersState::empty(),
        cursor: (0.0, 0.0),
        dragging: false,
        clipboard: arboard::Clipboard::new().ok(),
        outgoing,
        snapshot,
        error: None,
        clock: MonotonicClock::new(),
        deadline: None,
        redraw_pending: false,
        wsi_retry_pending: false,
    };
    events
        .run_app(&mut host)
        .map_err(|error| error.to_string())?;
    host.error.take().map_or(Ok(()), Err)
}
struct Host {
    views: SessionViews,
    appearance: misa_pixel_document::appearance::Choice,
    appearance_writer: std::sync::mpsc::SyncSender<misa_pixel_document::appearance::Choice>,
    window: Option<Arc<Window>>,
    surface: Option<misa_skia_vulkan::WindowRenderer>,
    modifiers: ModifiersState,
    cursor: (f32, f32),
    dragging: bool,
    clipboard: Option<arboard::Clipboard>,
    outgoing: Option<tokio::sync::mpsc::Sender<Action>>,
    snapshot: Option<String>,
    error: Option<String>,
    clock: MonotonicClock,
    deadline: Option<Duration>,
    redraw_pending: bool,
    wsi_retry_pending: bool,
}
impl Drop for Host {
    fn drop(&mut self) {
        self.surface.take(); // Vulkan surface and swapchain must die before their native window.
    }
}
impl Host {
    fn input(&mut self, event: Event) {
        let commands = self.views.input(event, self.clock.elapsed(), self.cursor);
        self.commands(commands);
    }
    fn key(&mut self, key: Key) {
        self.input(Event::Key(key));
    }
    fn text(&mut self, text: String) {
        self.input(Event::Text(text));
    }
    fn redraw(&mut self) {
        // An external event grants a fresh, bounded WSI retry opportunity.
        self.wsi_retry_pending = false;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
    fn commands(&mut self, commands: Vec<Action>) {
        for command in commands {
            let Some(command) = self.views.route(command) else {
                continue;
            };
            if let Action::Appearance(choice) = command {
                self.appearance = choice;
                self.views.appearance(choice);
                if self.appearance_writer.try_send(choice).is_err() {
                    self.views
                        .notice("Appearance storage is busy; choice was not saved");
                }
            } else if let Action::Ui(Command::Copy(text)) = command {
                self.views.notice(&match self
                    .clipboard
                    .as_mut()
                    .ok_or_else(|| "No clipboard is available".to_string())
                    .and_then(|clipboard| {
                        clipboard.set_text(text).map_err(|error| error.to_string())
                    }) {
                    Ok(()) => "Copied selection".into(),
                    Err(error) => error,
                });
            } else if let Some(outgoing) = &self.outgoing {
                if let Err(error) = outgoing.try_send(command) {
                    let reason = "The session is busy or disconnected".to_string();
                    match error.into_inner() {
                        Action::Ui(Command::Intent(
                            misa_kit::intent::Intent::Prompt { text, .. }
                            | misa_kit::intent::Intent::Interrupt { text, .. },
                        )) => self.views.reject_prompt(text, reason),
                        _ => self.views.notice(&reason),
                    }
                }
            } else {
                self.views.notice("Connect to a session to use this action");
            }
        }
        self.redraw();
    }
    fn paint(&mut self) -> Result<(), String> {
        let Some(window) = &self.window else {
            return Ok(());
        };
        let size = window.inner_size();
        self.deadline = None;
        self.redraw_pending = false;
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let elapsed = self.clock.elapsed();
        let light = self
            .appearance
            .light(window.theme() == Some(winit::window::Theme::Light));
        let colors = misa_pixel_document::appearance::Palette::new(light);
        let (scene, deadline) = self.views.frame(
            Size {
                width: size.width,
                height: size.height,
            },
            elapsed,
            light,
            self.appearance,
            colors,
        )?;
        self.deadline = deadline;
        let surface = self.surface.as_mut().ok_or("No window surface")?;
        if let Some(path) = &self.snapshot {
            surface
                .renderer()
                .render(&scene, colors.background)?
                .save(path)
                .map_err(|error| error.to_string())?;
        }
        let outcome = surface.present(&scene, colors.background, size.width, size.height)?;
        if retry_once(outcome.needs_redraw(), &mut self.wsi_retry_pending) {
            // Do not go through redraw(): it resets the external-event retry budget.
            window.request_redraw();
        }
        Ok(())
    }
}
/// At most one self-requested redraw per external event, even for persistent OUT_OF_DATE.
fn retry_once(needs_redraw: bool, pending: &mut bool) -> bool {
    if !needs_redraw {
        *pending = false;
        return false;
    }
    if *pending {
        return false;
    }
    *pending = true;
    true
}

impl ApplicationHandler<Update> for Host {
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.surface.take();
        self.window.take();
    }
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
                                "misa · pixels · Ctrl+O daemons · Ctrl+R requests · Ctrl+I panels · Ctrl+Shift+I appearance · Ctrl+Shift+P installed commands",
                            )
                            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 720.0)),
                    )
                    .map_err(|error| error.to_string())?,
            );
            window.set_ime_allowed(true);
            self.surface = Some(misa_skia_vulkan::WindowRenderer::new(
                window.display_handle().map_err(|e| e.to_string())?.as_raw(),
                window.window_handle().map_err(|e| e.to_string())?.as_raw(),
            )?);
            self.window = Some(window);
            self.redraw();
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.error = Some(error);
            events.exit();
        }
    }
    fn user_event(&mut self, _events: &ActiveEventLoop, update: Update) {
        match self.views.update(update) {
            StateChange::None => {}
            StateChange::Redraw => self.redraw(),
            StateChange::Selected(daemon) => {
                if let Some(window) = &self.window {
                    window.set_title(&format!("misa · pixels · {} · Ctrl+O daemons · Ctrl+R requests · Ctrl+Shift+I appearance · Ctrl+Shift+P installed commands", daemon.chars().take(12).collect::<String>()));
                }
                self.redraw();
            }
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
            WindowEvent::Resized(size) => self.input(Event::Resize(Size {
                width: size.width,
                height: size.height,
            })),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                if self.dragging {
                    self.input(Event::Pointer {
                        x: self.cursor.0,
                        y: self.cursor.1,
                        dragging: true,
                    });
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
                if self.dragging {
                    self.input(Event::Pointer {
                        x: self.cursor.0,
                        y: self.cursor.1,
                        dragging: false,
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
                    MouseScrollDelta::PixelDelta(position) => -position.y as f32,
                };
                self.input(Event::Wheel { delta });
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.text(text),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command = self.modifiers.control_key() || self.modifiers.super_key();
                match event.logical_key {
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("o") => {
                        self.views.open_chooser();
                        self.redraw();
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("r") => {
                        self.views.open_requests();
                        self.redraw();
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("i") => {
                        if self.modifiers.shift_key() {
                            self.views.open_presentations();
                            self.redraw();
                            return;
                        }
                        if self.views.cycle_panel() {
                            self.redraw();
                        }
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("p") => {
                        if self.modifiers.shift_key() {
                            self.views.open_commands();
                            self.redraw();
                        } else {
                            self.key(Key::Commands)
                        }
                    }
                    WinitKey::Named(NamedKey::ArrowUp) => self.key(Key::Up),
                    WinitKey::Named(NamedKey::ArrowDown) => self.key(Key::Down),
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("c") => {
                        self.key(Key::Copy)
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("a") => {
                        self.key(Key::SelectAll)
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("v") => {
                        if let Some(clipboard) = &mut self.clipboard {
                            if let Ok(text) = clipboard.get_text() {
                                self.text(text);
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
                    WinitKey::Character(text) if !command => self.text(text.to_string()),
                    WinitKey::Named(NamedKey::Space) if !command => self.text(" ".into()),
                    _ => {}
                }
            }
            WindowEvent::ThemeChanged(theme) => self.input(Event::Theme {
                light: self.appearance.light(theme == winit::window::Theme::Light),
            }),
            _ => {}
        }
    }

    /// Wake at the next phase boundary only while a visible app is animating.
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        if let Some(elapsed) = self.deadline {
            let deadline = self.clock.instant(elapsed);
            if !self.redraw_pending && Instant::now() >= deadline {
                self.redraw_pending = true;
                self.redraw();
            }
            events.set_control_flow(if self.redraw_pending {
                winit::event_loop::ControlFlow::Wait
            } else {
                winit::event_loop::ControlFlow::WaitUntil(deadline)
            });
        } else {
            events.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
    }
}

#[cfg(test)]
mod pulse_tests {
    use super::*;

    #[test]
    fn wsi_retry_is_bounded_until_external_redraw() {
        let mut pending = false;
        assert!(retry_once(true, &mut pending));
        assert!(!retry_once(true, &mut pending));
        pending = false; // input/resize calls Host::redraw
        assert!(retry_once(true, &mut pending));
        assert!(!retry_once(false, &mut pending));
        assert!(!pending);
    }

    #[test]
    fn deadline_is_anchored_and_strictly_after_painted_phase() {
        let start = Instant::now();
        for (elapsed, next) in [(0, 160), (159, 160), (160, 320), (479, 480), (960, 1120)] {
            assert_eq!(
                start
                    + misa_window_core::next_deadline(
                        Duration::from_millis(elapsed),
                        misa_pixel_document::ui::PULSE_PERIOD
                    ),
                start + Duration::from_millis(next)
            );
        }
        assert_eq!(
            start
                + misa_window_core::next_deadline(
                    Duration::from_micros(160_001),
                    misa_pixel_document::ui::PULSE_PERIOD
                ),
            start + Duration::from_millis(320)
        );
    }
}
