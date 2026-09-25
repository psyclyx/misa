//! A real desktop window consuming the same Skia scene used by PNG export.
use crate::app::{App, Command, Key};
use crate::connection::{self, Update};
use crate::workspace::Action;
use misa_window_core::{Clock, Event, MonotonicClock, Size};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
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
    let mut local = crate::workspace::Local::new(metrics.clone());
    if show_chooser {
        local.open_chooser();
    }
    let mut host = Host {
        appearance: crate::preferences::appearance(),
        appearance_writer: crate::preferences::appearance_writer(events.create_proxy()),
        app: App::new(view, metrics.clone()),
        metrics,
        local,
        directories: vec![],
        generation: 0,
        active: None,
        attention: None,
        parked: Default::default(),
        panels: Default::default(),
        panel: "status".into(),
        panel_focus: false,
        panel_top: f32::MAX,
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
    metrics: Arc<dyn misa_pixel_ui::TextMetrics>,
    appearance: crate::appearance::Choice,
    appearance_writer: std::sync::mpsc::SyncSender<crate::appearance::Choice>,
    app: App,
    local: crate::workspace::Local,
    directories: Vec<crate::workspace::DaemonChoice>,
    generation: u64,
    active: Option<(String, misa_proto::observation::Scope)>,
    attention: Option<(String, misa_proto::observation::Scope, String, i64)>,
    parked: std::collections::BTreeMap<
        (String, misa_proto::observation::Scope),
        (
            App,
            crate::workspace::Local,
            std::collections::BTreeMap<String, App>,
            u64,
        ),
    >,
    panels: std::collections::BTreeMap<String, App>,
    panel: String,
    panel_focus: bool,
    panel_top: f32,
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
        let elapsed = self.clock.elapsed();
        let commands = match event {
            Event::Pointer { x, y, dragging } => {
                if let Some(commands) = self.local.pointer(x, y, dragging) {
                    commands
                } else {
                    self.panel_focus = y >= self.panel_top;
                    if self.panel_focus {
                        self.panels
                            .get_mut(&self.panel)
                            .map(|panel| {
                                panel
                                    .drive(
                                        Event::Pointer {
                                            x,
                                            y: y - self.panel_top,
                                            dragging,
                                        },
                                        elapsed,
                                    )
                                    .commands
                                    .into_iter()
                                    .map(Action::Ui)
                                    .collect()
                            })
                            .unwrap_or_default()
                    } else {
                        self.app
                            .drive(Event::Pointer { x, y, dragging }, elapsed)
                            .commands
                            .into_iter()
                            .map(Action::Ui)
                            .collect()
                    }
                }
            }
            Event::Wheel { delta } => {
                if !self.local.scroll(delta) {
                    if self.cursor.1 >= self.panel_top {
                        if let Some(panel) = self.panels.get_mut(&self.panel) {
                            panel.drive(Event::Wheel { delta }, elapsed);
                        }
                    } else {
                        self.app.drive(Event::Wheel { delta }, elapsed);
                    }
                }
                vec![]
            }
            event @ (Event::Key(_) | Event::Text(_)) => self.input_event(event, elapsed),
            Event::Resize(_) | Event::Theme { .. } => vec![],
            Event::Redraw(_) => unreachable!("paint handles redraw"),
        };
        self.commands(commands);
    }
    fn key(&mut self, key: Key) {
        self.input(Event::Key(key));
    }
    fn text(&mut self, text: String) {
        self.input(Event::Text(text));
    }
    fn input_event(&mut self, event: Event, elapsed: Duration) -> Vec<Action> {
        if let Some(commands) = self.local.input(event.clone()) {
            commands
        } else if self.panel_focus {
            self.panels
                .get_mut(&self.panel)
                .map(|app| {
                    app.drive(event, elapsed)
                        .commands
                        .into_iter()
                        .map(Action::Ui)
                        .collect()
                })
                .unwrap_or_default()
        } else {
            self.app
                .drive(event, elapsed)
                .commands
                .into_iter()
                .map(Action::Ui)
                .collect()
        }
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
            let command = if let Action::SelectRequest {
                daemon,
                scope,
                request,
                generation,
            } = command
            {
                if self.active.as_ref() == Some(&(daemon.clone(), scope.clone())) {
                    self.local.open_request(request, generation);
                    continue;
                }
                self.attention = Some((daemon.clone(), scope.clone(), request, generation));
                Action::Select { daemon, scope }
            } else {
                command
            };
            if let Action::Appearance(choice) = command {
                self.appearance = choice;
                self.local.appearance(choice);
                if self.appearance_writer.try_send(choice).is_err() {
                    self.local
                        .notice("Appearance storage is busy; choice was not saved");
                }
            } else if let Action::Ui(Command::Copy(text)) = command {
                self.app.notice(&match self
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
                        )) => self.app.reject_prompt(text, reason),
                        _ => self.app.notice(&reason),
                    }
                }
            } else {
                self.app.notice("Connect to a session to use this action");
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
        let colors = crate::appearance::Palette::new(light);
        self.app.drive(Event::Theme { light }, elapsed);
        self.local.appearance(self.appearance);
        self.local.set_light(light);
        let scene = if let Some((scene, deadline)) =
            self.local.frame_at(size.width, size.height, elapsed)
        {
            self.deadline = deadline;
            scene
        } else {
            let panel_height = if self.panels.is_empty() || size.height < 6 {
                0
            } else {
                (size.height / 3).min(220)
            };
            self.panel_top = (size.height - panel_height) as f32;
            let main = self.app.drive(
                Event::Redraw(Size {
                    width: size.width,
                    height: size.height - panel_height,
                }),
                elapsed,
            );
            self.deadline = main.deadline;
            let mut scene = main.frame.ok_or("Empty main scene")?;
            scene.height = size.height as f32;
            if !self.panels.contains_key(&self.panel) {
                if let Some(id) = self.panels.keys().next() {
                    self.panel = id.clone();
                }
            }
            if let Some(panel) = (panel_height > 0)
                .then(|| self.panels.get_mut(&self.panel))
                .flatten()
            {
                panel.drive(Event::Theme { light }, elapsed);
                let pane_output = panel.drive(
                    Event::Redraw(Size {
                        width: size.width,
                        height: panel_height,
                    }),
                    elapsed,
                );
                self.deadline = self.deadline.or(pane_output.deadline);
                let pane = pane_output.frame.ok_or("Empty panel scene")?;
                scene.ops.push(misa_pixel_ui::Op::Rect {
                    x: 0.0,
                    y: self.panel_top,
                    width: size.width as f32,
                    height: panel_height as f32,
                    style: misa_style::Style {
                        fg: colors.background,
                        ..Default::default()
                    },
                });
                scene.ops.push(misa_pixel_ui::Op::Group {
                    x: 0.0,
                    y: self.panel_top,
                    ops: Arc::new(pane.ops),
                });
            }
            scene
        };
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
        match update {
            Update::Session { generation, update } => {
                if generation == self.generation {
                    apply_update(
                        &mut self.app,
                        &mut self.local,
                        &mut self.panels,
                        *update,
                        &self.metrics,
                    );
                    self.redraw();
                } else if let Some((app, local, panels, _)) = self
                    .parked
                    .values_mut()
                    .find(|(_, _, _, id)| *id == generation)
                {
                    apply_update(app, local, panels, *update, &self.metrics);
                }
                return;
            }
            Update::Selected {
                generation,
                daemon,
                scope,
            } => {
                let key = (daemon, scope);
                if self.active.as_ref() == Some(&key) && self.generation == generation {
                    return;
                }
                let old_app = std::mem::replace(
                    &mut self.app,
                    App::new(
                        misa_proto::Node::section("connecting"),
                        self.metrics.clone(),
                    ),
                );
                self.local.deactivate();
                let old_local = std::mem::replace(
                    &mut self.local,
                    crate::workspace::Local::new(self.metrics.clone()),
                );
                if let Some(old) = self.active.take() {
                    self.parked.insert(
                        old,
                        (
                            old_app,
                            old_local,
                            std::mem::take(&mut self.panels),
                            self.generation,
                        ),
                    );
                }
                if let Some((app, local, panels, _)) = self.parked.remove(&key) {
                    self.app = app;
                    self.local = local;
                    self.panels = panels;
                }
                self.local.directory(self.directories.clone());
                if let Some((daemon, scope, id, generation)) = self.attention.take() {
                    if key == (daemon, scope) {
                        self.local.open_request(id, generation);
                    }
                }
                self.active = Some(key);
                self.generation = generation;
                self.panel_focus = false;
                if let (Some(window), Some((daemon, _))) = (&self.window, &self.active) {
                    window.set_title(&format!("misa · pixels · {} · Ctrl+O daemons · Ctrl+R requests · Ctrl+Shift+I appearance · Ctrl+Shift+P installed commands", daemon.chars().take(12).collect::<String>()));
                }
            }
            Update::Directory(delivery) => {
                self.directories = delivery.capture();
                self.local.directory(self.directories.clone());
            }
            Update::ClosedInstance { daemon, scope } => {
                let key = (daemon, scope);
                self.parked.remove(&key);
                if self.active.as_ref() == Some(&key) {
                    self.active = None;
                    self.generation = 0;
                    self.app = App::new(misa_proto::Node::section("session"), self.metrics.clone());
                    self.panels.clear();
                    self.local = crate::workspace::Local::new(self.metrics.clone());
                    self.local.directory(self.directories.clone());
                    self.local.open_chooser();
                }
            }
            update => apply_update(
                &mut self.app,
                &mut self.local,
                &mut self.panels,
                update,
                &self.metrics,
            ),
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
                        self.local.open_chooser();
                        self.redraw();
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("r") => {
                        self.local.open_requests();
                        self.redraw();
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("i") => {
                        if self.modifiers.shift_key() {
                            self.local.open_presentations();
                            self.redraw();
                            return;
                        }
                        let ids: Vec<_> = self.panels.keys().cloned().collect();
                        if !ids.is_empty() {
                            let current = ids.iter().position(|id| id == &self.panel).unwrap_or(0);
                            self.panel = ids[(current + 1) % ids.len()].clone();
                            self.panel_focus = true;
                            self.redraw();
                        }
                    }
                    WinitKey::Character(ref text) if command && text.eq_ignore_ascii_case("p") => {
                        if self.modifiers.shift_key() {
                            self.local.open_commands();
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

/// Translate the production replica's validated transaction at the window boundary.
/// Do not materialize the document on each append: App retains unaffected owners.
fn document_update(
    update: &misa_client::document::Update,
) -> Option<crate::app::DocumentUpdate<'_>> {
    use crate::app::DocumentUpdate;
    use misa_client::document::Update;
    use misa_protocol::observation::{Applied, MemberChange, Status};
    match update {
        Update::Reset(document) => Some(DocumentUpdate::Reset {
            tree: &document.tree,
            streams: &document.streams,
        }),
        Update::Changed { member, applied } => match applied.as_ref() {
            Applied::Changed(members) => match members.get(member) {
                Some(MemberChange::Document {
                    tree,
                    live,
                    reset_live,
                }) => Some(DocumentUpdate::Changed {
                    tree,
                    live,
                    reset_live: *reset_live,
                }),
                _ => None,
            },
            _ => None,
        },
        Update::Unavailable(fault) => Some(DocumentUpdate::Notice(&fault.message)),
        Update::Status(status) => Some(DocumentUpdate::Notice(match status {
            Status::Awaiting => "Loading session…",
            Status::Current => "",
            Status::Recovering(_) => "Refreshing session…",
            Status::Stale(fault) | Status::Closed(fault) => &fault.message,
        })),
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
                        crate::app::PULSE_PERIOD
                    ),
                start + Duration::from_millis(next)
            );
        }
        assert_eq!(
            start
                + misa_window_core::next_deadline(
                    Duration::from_micros(160_001),
                    crate::app::PULSE_PERIOD
                ),
            start + Duration::from_millis(320)
        );
    }
}

#[cfg(test)]
mod document_adapter_tests {
    use super::*;
    use crate::app::DocumentUpdate;
    use misa_client::document::Update as DocumentDelivery;
    use misa_proto::sync::{Stream, StreamUpdate, Version, ViewOp};
    use misa_protocol::observation::{Applied, MemberChange, Status};

    #[test]
    fn borrows_reset_and_changed_payloads() {
        let reset = DocumentDelivery::Reset(misa_proto::observation::Document {
            version: Version {
                epoch: "test".into(),
                rev: 0,
            },
            tree: misa_proto::Node::section("session"),
            streams: vec![Stream {
                id: "live".into(),
                role: "message".into(),
                text: "hi".into(),
            }],
        });
        let DocumentDelivery::Reset(document) = &reset else {
            unreachable!()
        };
        let Some(DocumentUpdate::Reset { tree, streams }) = document_update(&reset) else {
            panic!("reset must reach App")
        };
        assert!(std::ptr::eq(tree, &document.tree));
        assert!(std::ptr::eq(streams, document.streams.as_slice()));

        let changed = DocumentDelivery::Changed {
            member: "body".into(),
            applied: Arc::new(Applied::Changed(std::collections::BTreeMap::from([(
                "body".into(),
                MemberChange::Document {
                    tree: vec![ViewOp::Remove { id: "old".into() }],
                    live: vec![StreamUpdate::End { id: "live".into() }],
                    reset_live: true,
                },
            )]))),
        };
        let DocumentDelivery::Changed { applied, .. } = &changed else {
            unreachable!()
        };
        let Applied::Changed(members) = applied.as_ref() else {
            unreachable!()
        };
        let MemberChange::Document { tree, live, .. } = &members["body"] else {
            unreachable!()
        };
        let Some(DocumentUpdate::Changed {
            tree: borrowed_tree,
            live: borrowed_live,
            reset_live,
        }) = document_update(&changed)
        else {
            panic!("changed member must reach App")
        };
        assert!(std::ptr::eq(borrowed_tree, tree.as_slice()));
        assert!(std::ptr::eq(borrowed_live, live.as_slice()));
        assert!(reset_live);
    }

    #[test]
    fn status_and_fault_notices_survive_translation() {
        for (status, expected) in [
            (Status::Awaiting, "Loading session…"),
            (Status::Current, ""),
            (Status::Recovering("retry".into()), "Refreshing session…"),
            (Status::Stale(misa_proto::Fault::query("stale")), "stale"),
            (Status::Closed(misa_proto::Fault::query("closed")), "closed"),
        ] {
            assert!(
                matches!(document_update(&DocumentDelivery::Status(status)), Some(DocumentUpdate::Notice(message)) if message == expected)
            );
        }
        let unavailable = DocumentDelivery::Unavailable(misa_proto::Fault::query("unavailable"));
        assert!(matches!(
            document_update(&unavailable),
            Some(DocumentUpdate::Notice("unavailable"))
        ));
    }
}

fn apply_update(
    app: &mut App,
    local: &mut crate::workspace::Local,
    panels: &mut std::collections::BTreeMap<String, App>,
    update: Update,
    metrics: &Arc<dyn misa_pixel_ui::TextMetrics>,
) {
    match update {
        Update::DaemonForm {
            daemon,
            form,
            drafts,
        } => local.prepared_daemon_form(daemon, form, drafts),
        Update::DocumentReport(document) => local.report(document),
        Update::InstalledCommands(commands) => local.commands(commands),
        Update::Composition {
            catalog,
            preferences,
            slots,
            observation,
        } => {
            panels.retain(|id, _| slots.contains(id));
            local.composition(catalog, preferences, observation);
        }
        Update::Form(form) => local.form(form),
        Update::Request {
            id,
            generation,
            model,
        } => {
            local.request(id, generation, model);
            if local.pending_requests() > 0 {
                app.notice(&format!(
                    "{} pending request(s) · Ctrl+R opens locally",
                    local.pending_requests()
                ));
            }
        }
        Update::Documents(deliveries) => {
            if local.observation != deliveries.first().map(|delivery| delivery.observation_id()) {
                return;
            }
            for (slot, update) in connection::Delivery::capture_many(&deliveries) {
                let app = if slot == "conversation" {
                    &mut *app
                } else {
                    panels.entry(slot).or_insert_with(|| {
                        let mut app =
                            App::new(misa_proto::Node::section("presentation"), metrics.clone());
                        app.scroll(-f32::MAX);
                        app
                    })
                };
                if let Some(update) = document_update(&update)
                    && let Err(error) = app.observed(&update)
                {
                    app.notice(&error);
                }
            }
        }
        Update::Shortcuts(commands) => app.declare_commands(commands),
        Update::Image { hash, image, .. } => {
            app.image(hash.clone(), image.clone());
            for panel in panels.values_mut() {
                panel.image(hash.clone(), image.clone());
            }
        }
        Update::Report { title, value } => app.report(title, value),
        Update::RejectedDraft { text, reason } => app.reject_prompt(text, reason),
        Update::Notice(notice) => {
            local.notice(&notice);
            app.notice(&notice);
        }
        Update::Session { .. }
        | Update::Selected { .. }
        | Update::Directory(_)
        | Update::ClosedInstance { .. } => {
            unreachable!("relationship events cannot be session payloads")
        }
    }
}
