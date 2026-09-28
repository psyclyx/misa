//! Session-owned document, local overlays, and presentation panel state.
use crate::connection::{self, Update};
use crate::workspace::{Action, DaemonChoice, Local};
use misa_pixel_document::ui::DocumentUi;
use misa_window_core::{Event, Size};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

type ScopeKey = (String, misa_proto::observation::Scope);

struct ParkedSession {
    app: DocumentUi,
    local: Local,
    panels: BTreeMap<String, DocumentUi>,
    panel: String,
    panel_focus: bool,
    panel_top: f32,
    generation: u64,
}

/// What the window needs to do after a relationship update.
pub(super) enum StateChange {
    None,
    Redraw,
    Selected(String),
}

pub(super) struct SessionViews {
    metrics: Arc<dyn misa_pixel_ui::TextMetrics>,
    app: DocumentUi,
    local: Local,
    directories: Vec<DaemonChoice>,
    generation: u64,
    active: Option<ScopeKey>,
    attention: Option<(ScopeKey, String, i64)>,
    parked: BTreeMap<ScopeKey, ParkedSession>,
    panels: BTreeMap<String, DocumentUi>,
    panel: String,
    panel_focus: bool,
    panel_top: f32,
    /// The panel's last known content height; panels are bar-sized documents.
    panel_height: f32,
    background_waker: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl SessionViews {
    pub(super) fn new(
        view: misa_proto::view::Node,
        metrics: Arc<dyn misa_pixel_ui::TextMetrics>,
        show_chooser: bool,
    ) -> Self {
        let mut local = Local::new(metrics.clone());
        if show_chooser {
            local.open_chooser();
        }
        Self {
            app: DocumentUi::new(view, metrics.clone()),
            metrics,
            local,
            directories: vec![],
            generation: 0,
            active: None,
            attention: None,
            parked: BTreeMap::new(),
            panels: BTreeMap::new(),
            panel: "status".into(),
            panel_focus: false,
            panel_top: f32::MAX,
            panel_height: 0.0,
            background_waker: None,
        }
    }
    /// Hand every presented document the host's generic poll wake. The callback
    /// may run on a layout worker and must only request a UI poll.
    pub(super) fn enable_background(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.background_waker = Some(waker);
        self.refresh_background();
    }
    /// Parked scopes and hidden panels keep their measurements but never run.
    pub(super) fn pause_background(&mut self) {
        self.background_waker = None;
        self.refresh_background();
    }
    pub(super) fn refresh_background(&mut self) {
        let waker = self.background_waker.clone();
        for scope in self.parked.values_mut() {
            scope.app.pause_background();
            for panel in scope.panels.values_mut() {
                panel.pause_background();
            }
        }
        let Some(waker) = waker else {
            self.app.pause_background();
            for panel in self.panels.values_mut() {
                panel.pause_background();
            }
            return;
        };
        self.app.enable_background(waker.clone());
        for (id, panel) in &mut self.panels {
            if id == &self.panel {
                panel.enable_background(waker.clone());
            } else {
                panel.pause_background();
            }
        }
    }
    /// Advance bounded prewarm work on the UI thread for presented documents.
    pub(super) fn poll_background(&mut self) -> usize {
        self.refresh_background();
        let mut installed = self.app.poll_background();
        if let Some(panel) = self.panels.get_mut(&self.panel) {
            installed += panel.poll_background();
        }
        installed
    }
    pub(super) fn notice(&mut self, message: &str) {
        self.app.notice(message);
    }
    pub(super) fn reject_prompt(&mut self, text: String, reason: String) {
        self.app.reject_prompt(text, reason);
    }
    pub(super) fn appearance(&mut self, choice: misa_pixel_document::appearance::Choice) {
        self.local.appearance(choice);
    }
    pub(super) fn open_chooser(&mut self) {
        self.local.open_chooser();
    }
    pub(super) fn open_requests(&mut self) {
        self.local.show_requests();
    }
    pub(super) fn open_presentations(&mut self) {
        self.local.open_presentations();
    }
    pub(super) fn open_commands(&mut self) {
        self.local.open_commands();
    }
    pub(super) fn cycle_panel(&mut self) -> bool {
        let ids: Vec<_> = self.panels.keys().cloned().collect();
        if ids.is_empty() {
            return false;
        }
        // The default "status" panel may not have arrived yet. Select the
        // first available panel instead of cycling from a nonexistent one.
        let next = ids
            .iter()
            .position(|id| id == &self.panel)
            .map_or(0, |current| (current + 1) % ids.len());
        self.panel = ids[next].clone();
        self.panel_focus = true;
        true
    }
    /// Route request attention locally or select the owner scope first.
    pub(super) fn route(&mut self, action: Action) -> Option<Action> {
        if let Action::SelectRequest {
            daemon,
            scope,
            request,
            generation,
        } = action
        {
            let key = (daemon, scope);
            if self.active.as_ref() == Some(&key) {
                self.local.show_request(request, generation);
                return None;
            }
            self.attention = Some((key.clone(), request, generation));
            return Some(Action::Select {
                daemon: key.0,
                scope: key.1,
            });
        }
        Some(action)
    }
    fn park(&mut self) {
        let key = self.active.take();
        self.local.deactivate();
        let mut parked = ParkedSession {
            app: std::mem::replace(
                &mut self.app,
                DocumentUi::new(
                    misa_proto::Node::section("connecting"),
                    self.metrics.clone(),
                ),
            ),
            local: std::mem::replace(&mut self.local, Local::new(self.metrics.clone())),
            panels: std::mem::take(&mut self.panels),
            panel: std::mem::replace(&mut self.panel, "status".into()),
            panel_focus: std::mem::replace(&mut self.panel_focus, false),
            panel_top: std::mem::replace(&mut self.panel_top, f32::MAX),
            generation: self.generation,
        };
        // A scope that is not on screen keeps its measurements but never runs.
        parked.app.pause_background();
        for panel in parked.panels.values_mut() {
            panel.pause_background();
        }
        if let Some(key) = key {
            self.parked.insert(key, parked);
        }
    }
    fn select(&mut self, key: ScopeKey, generation: u64) -> StateChange {
        if self.active.as_ref() == Some(&key) && self.generation == generation {
            return StateChange::None;
        }
        self.park();
        if let Some(saved) = self.parked.remove(&key) {
            self.app = saved.app;
            self.local = saved.local;
            self.panels = saved.panels;
            self.panel = saved.panel;
            self.panel_focus = saved.panel_focus;
            self.panel_top = saved.panel_top;
        }
        self.local.directory(self.directories.clone());
        if self
            .attention
            .as_ref()
            .is_some_and(|(target, _, _)| target == &key)
        {
            let (_, id, request_generation) = self.attention.take().expect("matching attention");
            self.local.show_request(id, request_generation);
        }
        let title = key.0.clone();
        self.active = Some(key);
        self.generation = generation;
        StateChange::Selected(title)
    }
    fn forget(&mut self, key: ScopeKey) {
        self.parked.remove(&key);
        if self
            .attention
            .as_ref()
            .is_some_and(|(target, _, _)| target == &key)
        {
            self.attention = None;
        }
        if self.active.as_ref() != Some(&key) {
            return;
        }
        self.active = None;
        self.generation = 0;
        self.app = DocumentUi::new(misa_proto::Node::section("session"), self.metrics.clone());
        self.panels.clear();
        self.panel = "status".into();
        self.panel_focus = false;
        self.panel_top = f32::MAX;
        self.local = Local::new(self.metrics.clone());
        self.local.directory(self.directories.clone());
        self.local.open_chooser();
    }
    pub(super) fn update(&mut self, update: Update) -> StateChange {
        match update {
            Update::Session { generation, update } => {
                if self.active.is_some() && generation == self.generation {
                    apply_update(
                        &mut self.app,
                        &mut self.local,
                        &mut self.panels,
                        *update,
                        &self.metrics,
                    );
                    StateChange::Redraw
                } else {
                    if let Some(saved) = self
                        .parked
                        .values_mut()
                        .find(|saved| saved.generation == generation)
                    {
                        apply_update(
                            &mut saved.app,
                            &mut saved.local,
                            &mut saved.panels,
                            *update,
                            &self.metrics,
                        );
                    }
                    StateChange::None
                }
            }
            Update::Selected {
                generation,
                daemon,
                scope,
            } => self.select((daemon, scope), generation),
            Update::Directory(delivery) => {
                self.directories = delivery.capture();
                self.local.directory(self.directories.clone());
                StateChange::Redraw
            }
            Update::ClosedInstance { daemon, scope } => {
                self.forget((daemon, scope));
                StateChange::Redraw
            }
            update => {
                apply_update(
                    &mut self.app,
                    &mut self.local,
                    &mut self.panels,
                    update,
                    &self.metrics,
                );
                StateChange::Redraw
            }
        }
    }
    pub(super) fn input(
        &mut self,
        event: Event,
        elapsed: Duration,
        cursor: (f32, f32),
    ) -> Vec<Action> {
        let commands = match event {
            Event::Pointer { x, y, phase } => {
                if let Some(commands) = self.local.pointer(x, y, phase) {
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
                                            phase,
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
                            .drive(Event::Pointer { x, y, phase }, elapsed)
                            .commands
                            .into_iter()
                            .map(Action::Ui)
                            .collect()
                    }
                }
            }
            Event::ContextMenu { x, y } => {
                if let Some(commands) = self.local.context_menu(x, y) {
                    commands
                } else if y >= self.panel_top {
                    self.panel_focus = true;
                    self.panels
                        .get_mut(&self.panel)
                        .map(|panel| {
                            panel
                                .drive(
                                    Event::ContextMenu {
                                        x,
                                        y: y - self.panel_top,
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
                    self.panel_focus = false;
                    self.app
                        .drive(Event::ContextMenu { x, y }, elapsed)
                        .commands
                        .into_iter()
                        .map(Action::Ui)
                        .collect()
                }
            }
            Event::Wheel { delta } => {
                if !self.local.scroll(delta) {
                    if cursor.1 >= self.panel_top {
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
        commands
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
    pub(super) fn frame(
        &mut self,
        size: Size,
        elapsed: Duration,
        light: bool,
        appearance: misa_pixel_document::appearance::Choice,
        colors: misa_pixel_document::appearance::Palette,
    ) -> Result<(misa_pixel_ui::Scene, Option<Duration>), String> {
        self.app.drive(Event::Theme { light }, elapsed);
        self.local.appearance(appearance);
        self.local.set_light(light);
        let (scene, next_deadline) = if let Some((scene, deadline)) =
            self.local.frame_at(size.width, size.height, elapsed)
        {
            (scene, deadline)
        } else {
            // The panel is a bar-sized document: it takes the height it needs,
            // not a third of the window. Unknown measurements keep the last
            // known height rather than flickering the panel away.
            let wanted = self
                .panels
                .get_mut(&self.panel)
                .and_then(misa_pixel_document::ui::DocumentUi::content_height);
            let panel_height = if self.panels.is_empty() || size.height < 6 {
                0
            } else {
                if let Some(height) = wanted {
                    self.panel_height = height;
                }
                self.panel_height.clamp(0.0, (size.height / 3) as f32) as u32
            };
            self.panel_top = (size.height - panel_height) as f32;
            let main = self.app.drive(
                Event::Redraw(Size {
                    width: size.width,
                    height: size.height - panel_height,
                }),
                elapsed,
            );
            let mut next_deadline = main.deadline;
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
                next_deadline = next_deadline.or(pane_output.deadline);
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
            (scene, next_deadline)
        };
        // Selection can settle inside the frame (a panel may arrive late), so
        // prewarm workers follow the presented documents afterwards.
        self.refresh_background();
        Ok((scene, next_deadline))
    }
}
/// Translate the production replica's validated transaction at the window boundary.
/// Do not materialize the document on each append: DocumentUi retains unaffected owners.
fn document_update(
    update: &misa_client::document::Update,
) -> Option<misa_pixel_document::ui::DocumentUpdate<'_>> {
    use misa_client::document::Update;
    use misa_pixel_document::ui::DocumentUpdate;
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
mod document_adapter_tests {
    use super::*;
    use misa_client::document::Update as DocumentDelivery;
    use misa_pixel_document::ui::DocumentUpdate;
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
            panic!("reset must reach DocumentUi")
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
            panic!("changed member must reach DocumentUi")
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

#[cfg(test)]
mod session_tests {
    use super::*;
    use misa_proto::observation::{Scope, ScopeId};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key(id: &str) -> ScopeKey {
        (
            "daemon".into(),
            Scope {
                id: ScopeId::Session { id: id.into() },
                incarnation: "one".into(),
            },
        )
    }

    #[test]
    fn only_presented_documents_run_workers_and_parked_scopes_pause() {
        let metrics = misa_skia_paint::text_metrics().unwrap();
        let mut views =
            SessionViews::new(misa_proto::Node::section("main"), metrics.clone(), false);
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        views.enable_background(waker);
        views.select(key("a"), 1);
        views.panels.insert(
            "status".into(),
            DocumentUi::new(misa_proto::Node::section("shown"), metrics.clone()),
        );
        views.panels.insert(
            "other".into(),
            DocumentUi::new(misa_proto::Node::section("hidden"), metrics),
        );
        views.refresh_background();
        assert!(views.app.background_work_pending());
        assert!(views.panels["status"].background_work_pending());
        assert!(!views.panels["other"].background_work_pending());
        views.select(key("b"), 2);
        let parked = views.parked.values().next().unwrap();
        assert!(!parked.app.background_work_pending());
        assert!(
            parked
                .panels
                .values()
                .all(|panel| !panel.background_work_pending())
        );
        views.pause_background();
        assert!(!views.app.background_work_pending());
        assert_eq!(views.poll_background(), 0);
    }

    #[test]
    fn polling_without_work_never_rewakes_the_host() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let metrics = misa_skia_paint::text_metrics().unwrap();
        let mut views = SessionViews::new(misa_proto::Node::section("main"), metrics, false);
        views.enable_background(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        for _ in 0..64 {
            views.poll_background();
        }
        assert!(
            wakes.load(Ordering::SeqCst) <= 4,
            "re-arming the same wake must not loop: {} wakes",
            wakes.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn cycling_before_the_default_panel_arrives_selects_an_available_panel() {
        let metrics = misa_skia_paint::text_metrics().unwrap();
        let mut views =
            SessionViews::new(misa_proto::Node::section("offline"), metrics.clone(), false);
        assert!(!views.cycle_panel());
        views.panels.insert(
            "inspector".into(),
            DocumentUi::new(misa_proto::Node::section("inspector"), metrics.clone()),
        );
        views.panels.insert(
            "requests".into(),
            DocumentUi::new(misa_proto::Node::section("requests"), metrics),
        );
        assert!(views.cycle_panel());
        assert_eq!(views.panel, "inspector");
        assert!(views.panel_focus);
        assert!(views.cycle_panel());
        assert_eq!(views.panel, "requests");
    }

    #[test]
    fn switching_with_panel_restores_selection_and_rejects_old_generation() {
        let metrics = misa_skia_paint::text_metrics().unwrap();
        let mut views =
            SessionViews::new(misa_proto::Node::section("offline"), metrics.clone(), false);
        let a = key("a");
        let b = key("b");
        views.panels.insert(
            "offline".into(),
            DocumentUi::new(misa_proto::Node::section("offline panel"), metrics.clone()),
        );
        assert!(matches!(
            views.select(a.clone(), 10),
            StateChange::Selected(_)
        ));
        assert!(!views.panels.contains_key("offline"));
        views.panels.insert(
            "inspector".into(),
            DocumentUi::new(misa_proto::Node::section("inspector"), metrics.clone()),
        );
        views.panels.insert(
            "status".into(),
            DocumentUi::new(misa_proto::Node::section("status"), metrics),
        );
        assert!(views.cycle_panel()); // status -> inspector
        views.panel_top = 500.0;
        let panel = &views.panels["inspector"] as *const DocumentUi;
        views.select(b.clone(), 20);
        assert!(views.panels.is_empty());
        assert_eq!(views.panel, "status");
        assert!(!views.panel_focus);
        // Queued delivery for the parked session is not a window redraw.
        assert!(matches!(
            views.update(Update::Session {
                generation: 10,
                update: Box::new(Update::Notice("parked".into()))
            }),
            StateChange::None
        ));
        views.select(a.clone(), 30);
        assert_eq!(views.panel, "inspector");
        assert!(views.panel_focus);
        assert_eq!(views.panel_top, 500.0);
        assert_eq!(&views.panels["inspector"] as *const DocumentUi, panel);
        // A replaced handle must not deliver even when the same scope is active again.
        let invalid = || Update::Selected {
            generation: 99,
            daemon: "wrong".into(),
            scope: b.1.clone(),
        };
        assert!(matches!(
            views.update(Update::Session {
                generation: 10,
                update: Box::new(invalid())
            }),
            StateChange::None
        ));
        assert!(matches!(
            views.update(Update::Session {
                generation: 30,
                update: Box::new(Update::Notice("current".into()))
            }),
            StateChange::Redraw
        ));
        views.forget(a);
        assert!(matches!(
            views.update(Update::Session {
                generation: 30,
                update: Box::new(invalid())
            }),
            StateChange::None
        ));
    }
}

fn apply_update(
    app: &mut DocumentUi,
    local: &mut crate::workspace::Local,
    panels: &mut std::collections::BTreeMap<String, DocumentUi>,
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
            local.requests.update(id, generation, model);
            if local.requests.len() > 0 {
                app.notice(&format!(
                    "{} pending request(s) · Ctrl+R opens locally",
                    local.requests.len()
                ));
            }
        }
        Update::Documents(deliveries) => {
            if local.observation() != deliveries.first().map(|delivery| delivery.observation_id()) {
                return;
            }
            for (slot, update) in connection::Delivery::capture_many(&deliveries) {
                let app = if slot == "conversation" {
                    &mut *app
                } else {
                    panels.entry(slot).or_insert_with(|| {
                        let mut app = DocumentUi::new(
                            misa_proto::Node::section("presentation"),
                            metrics.clone(),
                        );
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
