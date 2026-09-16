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
    live: bool,
) -> Result<(), String> {
    let events = EventLoop::<Update>::with_user_event()
        .build()
        .map_err(|error| error.to_string())?;
    let show_chooser = live && ticket.is_none();
    let outgoing = live.then(|| connection::start(ticket, events.create_proxy()));
    let mut local = crate::workspace::Local::default();
    if show_chooser {
        local.open_chooser();
    }
    let mut host = Host {
        appearance: crate::preferences::appearance(),
        appearance_writer: crate::preferences::appearance_writer(events.create_proxy()),
        app: App::new(view),
        local,
        directories: vec![],
        generation: 0,
        active: None,
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
    };
    events
        .run_app(&mut host)
        .map_err(|error| error.to_string())?;
    host.error.map_or(Ok(()), Err)
}
struct Host {
    appearance: crate::appearance::Choice,
    appearance_writer: std::sync::mpsc::SyncSender<crate::appearance::Choice>,
    app: App,
    local: crate::workspace::Local,
    directories: Vec<crate::workspace::DaemonChoice>,
    generation: u64,
    active: Option<(String, misa_proto::observation::Scope)>,
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
    fn pointer(&mut self, dragging: bool) -> Vec<Command> {
        if let Some(commands) = self.local.pointer(self.cursor.0, self.cursor.1, dragging) {
            return commands;
        }
        self.panel_focus = self.cursor.1 >= self.panel_top;
        if self.panel_focus {
            self.panels
                .get_mut(&self.panel)
                .map(|panel| panel.pointer(self.cursor.0, self.cursor.1 - self.panel_top, dragging))
                .unwrap_or_default()
        } else {
            self.app.pointer(self.cursor.0, self.cursor.1, dragging)
        }
    }
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
    fn commands(&mut self, commands: Vec<Command>) {
        for command in commands {
            if let Command::Appearance(choice) = command {
                self.appearance = choice;
                self.local.appearance(choice);
                if self.appearance_writer.try_send(choice).is_err() {
                    self.local
                        .notice("Appearance storage is busy; choice was not saved");
                }
            } else if let Command::Copy(text) = command {
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
                if let Err(error) = outgoing.try_send(command) {
                    let reason = "The session is busy or disconnected".to_string();
                    match error.into_inner() {
                        Command::Intent(
                            misa_proto::Intent::Prompt { text, .. }
                            | misa_proto::Intent::Interrupt { text, .. },
                        ) => self.app.reject_prompt(text, reason),
                        _ => self.app.notice = reason,
                    }
                }
            } else {
                self.app.notice = "Connect to a session to use this action".into();
            }
        }
        self.redraw();
    }
    fn key(&mut self, key: Key) {
        let commands = if let Some(commands) = self.local.key(key.clone()) {
            commands
        } else if self.panel_focus {
            self.panels
                .get_mut(&self.panel)
                .map(|app| app.key(key))
                .unwrap_or_default()
        } else {
            self.app.key(key)
        };
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
        let light = self
            .appearance
            .light(window.theme() == Some(winit::window::Theme::Light));
        let colors = crate::appearance::Palette::new(light);
        self.app.set_light(light);
        self.local.appearance(self.appearance);
        self.local.set_light(light);
        let scene = if let Some(scene) = self.local.frame(size.width, size.height) {
            scene
        } else {
            let panel_height = if self.panels.is_empty() {
                0
            } else {
                (size.height / 3).min(220)
            };
            self.panel_top = (size.height - panel_height) as f32;
            let mut scene = self.app.frame(size.width, size.height - panel_height);
            scene.height = size.height as f32;
            if !self.panels.contains_key(&self.panel) {
                if let Some(id) = self.panels.keys().next() {
                    self.panel = id.clone();
                }
            }
            if let Some(panel) = self.panels.get_mut(&self.panel) {
                panel.set_light(light);
                let pane = panel.frame(size.width, panel_height);
                scene.ops.push(crate::Op::Rect {
                    x: 0.0,
                    y: self.panel_top,
                    width: size.width as f32,
                    height: panel_height as f32,
                    style: misa_render::Style {
                        fg: colors.background,
                        ..Default::default()
                    },
                });
                scene.ops.push(crate::Op::Group {
                    x: 0.0,
                    y: self.panel_top,
                    ops: Arc::new(pane.ops),
                });
            }
            scene
        };
        let image = crate::paint::raster(&scene, colors.background)?;
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
                            .with_title(
                                "misa · pixels · Ctrl+O daemons · Ctrl+R requests · Ctrl+I panels · Ctrl+Shift+I appearance · Ctrl+Shift+P installed commands",
                            )
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
    fn user_event(&mut self, _events: &ActiveEventLoop, update: Update) {
        match update {
            Update::Session { generation, update } => {
                if generation == self.generation {
                    apply_update(&mut self.app, &mut self.local, &mut self.panels, *update);
                    self.redraw();
                } else if let Some((app, local, panels, _)) = self
                    .parked
                    .values_mut()
                    .find(|(_, _, _, id)| *id == generation)
                {
                    apply_update(app, local, panels, *update);
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
                    App::new(misa_proto::Node::section("connecting")),
                );
                self.local.deactivate();
                let old_local = std::mem::take(&mut self.local);
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
                    self.app = App::new(misa_proto::Node::section("session"));
                    self.panels.clear();
                    self.local = crate::workspace::Local::default();
                    self.local.directory(self.directories.clone());
                    self.local.open_chooser();
                }
            }
            update => apply_update(&mut self.app, &mut self.local, &mut self.panels, update),
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
                    let commands = self.pointer(true);
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
                    let commands = self.pointer(false);
                    self.commands(commands);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 60.0,
                    MouseScrollDelta::PixelDelta(position) => -position.y as f32,
                };
                if !self.local.scroll(delta) {
                    if self.cursor.1 >= self.panel_top {
                        if let Some(panel) = self.panels.get_mut(&self.panel) {
                            panel.scroll(delta);
                        }
                    } else {
                        self.app.scroll(delta);
                    }
                }
                self.redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.key(Key::Text(text)),
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
            WindowEvent::ThemeChanged(_) => self.redraw(),
            _ => {}
        }
    }
}

fn apply_update(
    app: &mut App,
    local: &mut crate::workspace::Local,
    panels: &mut std::collections::BTreeMap<String, App>,
    update: Update,
) {
    match update {
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
                app.notice = format!(
                    "{} pending request(s) · Ctrl+R opens locally",
                    local.pending_requests()
                );
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
                        let mut app = App::new(misa_proto::Node::section("presentation"));
                        app.scroll(-f32::MAX);
                        app
                    })
                };
                if let Err(error) = app.observed(&update) {
                    app.notice = error;
                }
            }
        }
        Update::Shortcuts(commands) => app.commands = commands,
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
            app.notice = notice;
        }
        Update::Session { .. }
        | Update::Selected { .. }
        | Update::Directory(_)
        | Update::ClosedInstance { .. } => {
            unreachable!("relationship events cannot be session payloads")
        }
    }
}
