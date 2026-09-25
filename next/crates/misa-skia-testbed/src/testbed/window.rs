//! Optional native-window adapter for the shared offline fixtures.
use super::{Fixtures, Mode};
use crate::app::Key;
use misa_render::Color;
use misa_window_core::{Clock, Event, MonotonicClock, Size};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

pub fn run() -> Result<(), String> {
    let events = EventLoop::new().map_err(|error| error.to_string())?;
    let mut host = Host {
        fixtures: Fixtures::new(),
        window: None,
        surface: None,
        modifiers: ModifiersState::empty(),
        cursor: (0.0, 0.0),
        error: None,
        clock: MonotonicClock::new(),
        redraw_pending: false,
        wsi_retry_pending: false,
    };
    events
        .run_app(&mut host)
        .map_err(|error| error.to_string())?;
    host.error.take().map_or(Ok(()), Err)
}

struct Host {
    fixtures: Fixtures,
    window: Option<Arc<Window>>,
    surface: Option<misa_skia_vulkan::WindowRenderer>,
    modifiers: ModifiersState,
    cursor: (f32, f32),
    error: Option<String>,
    clock: MonotonicClock,
    redraw_pending: bool,
    wsi_retry_pending: bool,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.surface.take(); // Drop Vulkan resources before the native window.
    }
}
impl Host {
    fn redraw(&mut self) {
        // Only external events reset the WSI retry budget.
        self.wsi_retry_pending = false;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn paint(&mut self) -> Result<(), String> {
        let size = self.window.as_ref().ok_or("No window")?.inner_size();
        if size.width == 0 || size.height == 0 {
            self.fixtures.deadline = None;
            self.redraw_pending = false;
            return Ok(());
        }
        let elapsed = self.clock.elapsed();
        let scene = self
            .fixtures
            .frame_at(size.width, size.height, Some(elapsed));
        let outcome = self.surface.as_mut().ok_or("No window surface")?.present(
            &scene,
            Color::Rgb(20, 22, 26),
            size.width,
            size.height,
        )?;
        if retry_once(outcome.needs_redraw(), &mut self.wsi_retry_pending) {
            // Bypass redraw(): this retry must not replenish its own budget.
            self.window.as_ref().unwrap().request_redraw();
        }
        Ok(())
    }

    fn input(&mut self, event: Event) {
        let width = self
            .window
            .as_ref()
            .map_or(0, |window| window.inner_size().width);
        self.fixtures.input(event, self.clock.elapsed(), width);
        self.redraw();
    }

    fn keyboard(&mut self, key: WinitKey) {
        let command = self.modifiers.control_key() || self.modifiers.super_key();
        let event = match key {
            WinitKey::Character(ref text) if command && (text == "1" || text == "2") => {
                self.fixtures.select(text);
                self.redraw();
                return;
            }
            WinitKey::Named(NamedKey::Enter) => Event::Key(Key::Enter {
                newline: self.modifiers.shift_key(),
            }),
            WinitKey::Named(NamedKey::Tab) => Event::Key(Key::Tab {
                backward: self.modifiers.shift_key(),
            }),
            WinitKey::Named(NamedKey::Backspace) => Event::Key(Key::Backspace),
            WinitKey::Named(NamedKey::Delete) => Event::Key(Key::Delete),
            WinitKey::Named(NamedKey::ArrowLeft) => Event::Key(Key::Left),
            WinitKey::Named(NamedKey::ArrowRight) => Event::Key(Key::Right),
            WinitKey::Named(NamedKey::Home) => Event::Key(Key::Home),
            WinitKey::Named(NamedKey::End) => Event::Key(Key::End),
            WinitKey::Named(NamedKey::Escape) => Event::Key(Key::Escape),
            WinitKey::Named(NamedKey::Space) if !command => Event::Text(" ".into()),
            WinitKey::Character(text) if !command => Event::Text(text.to_string()),
            _ => return,
        };
        self.input(event);
    }
}

/// Bound self-requested WSI retries; a later input/resize may start a new attempt.
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

impl ApplicationHandler for Host {
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
                                "misa · local Skia testbed · Ctrl+1 native · Ctrl+2 semantic",
                            )
                            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 640.0)),
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

    fn window_event(&mut self, events: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => events.exit(),
            WindowEvent::RedrawRequested => {
                self.redraw_pending = false;
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
                self.cursor = (position.x as f32, position.y as f32)
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.input(Event::Pointer {
                    x: self.cursor.0,
                    y: self.cursor.1,
                    dragging: false,
                });
            }
            WindowEvent::MouseWheel { delta, .. } if self.fixtures.mode == Mode::Semantic => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
                    MouseScrollDelta::PixelDelta(position) => -position.y as f32,
                };
                self.input(Event::Wheel { delta });
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.input(Event::Text(text));
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.keyboard(event.logical_key)
            }
            WindowEvent::ThemeChanged(theme) => self.input(Event::Theme {
                light: theme == winit::window::Theme::Light,
            }),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        if let Some(deadline) = self
            .fixtures
            .pulse_deadline(self.clock.instant(Duration::ZERO))
        {
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
mod tests {
    use super::*;
    #[test]
    fn wsi_retry_does_not_spin_when_idle() {
        let mut pending = false;
        assert!(retry_once(true, &mut pending));
        assert!(!retry_once(true, &mut pending));
        pending = false; // a later input/resize calls Host::redraw
        assert!(retry_once(true, &mut pending));
        assert!(!retry_once(false, &mut pending));
        assert!(!pending);
    }
}
