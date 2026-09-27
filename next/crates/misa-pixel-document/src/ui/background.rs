//! Bounded, protocol-free owner prewarm. The adapter alone reads or mutates live
//! UI state; the worker sees only immutable owner snapshots and returns exact
//! measurements. Heights always land in the viewport's measurement index,
//! display lists only for owners adjacent to the visible flow, and no result
//! ever requires a host to paint another frame.
use super::{
    DocumentUi,
    flow::FlowId,
    measurement::{self, OwnerSnapshot, SnapshotError},
    retained::RetainedScenes,
};
use misa_pixel_ui::{FlowPosition, TextMetrics};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};

#[cfg(test)]
#[path = "background_tests.rs"]
mod tests;

/// Jobs in flight, and results awaiting the next UI poll. Two bounds how much
/// detached layout work a pause or drop can leave behind.
const CAPACITY: usize = 2;
/// Flow steps one poll may walk before returning to the host. Skips over
/// already measured owners are cheap index navigation; only capture attempts
/// (`WALK_BUDGET`) do measurement work.
const SKIP_BUDGET: usize = 512;
/// Capture attempts one poll may make before returning to the host.
const WALK_BUDGET: usize = CAPACITY * 4;
/// Owners adjacent to the visible flow keep their display list; the rest keep
/// only their exact height. A cursor restart grants a fresh set of slots.
const SCENE_PREWARM: usize = 8;
/// Serialized snapshot input one poll may capture across all owners.
const CAPTURE_BUDGET: usize = 3 * super::measurement::SNAPSHOT_BUDGET;

struct Job {
    id: FlowId,
    epoch: u64,
    width: f32,
    style_generation: u64,
    keep_scene: bool,
    snapshot: OwnerSnapshot,
}

struct ResultOwner {
    id: FlowId,
    epoch: u64,
    width: f32,
    style_generation: u64,
    height: f32,
    retained: Option<RetainedScenes>,
}

fn step(job: Job, epoch: &AtomicU64, metrics: &Arc<dyn TextMetrics>) -> ResultOwner {
    let cancelled = epoch.load(Ordering::Acquire) != job.epoch;
    let (height, retained) = if cancelled {
        (0.0, None)
    } else {
        let rendered = measurement::render(job.snapshot, Arc::clone(metrics));
        (rendered.height, job.keep_scene.then_some(rendered.retained))
    };
    ResultOwner {
        id: job.id,
        epoch: job.epoch,
        width: job.width,
        style_generation: job.style_generation,
        height,
        retained,
    }
}

/// Coalescing host wake. The callback may run on the worker thread and must do
/// nothing but request a UI poll; it must never touch the document directly.
#[derive(Default)]
struct Wake {
    pending: AtomicBool,
    waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}
impl Wake {
    fn notify(&self) {
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(waker) = self.waker.lock().expect("layout waker").as_ref() {
            waker();
        }
    }
    /// Arming the same host wake twice is a no-op: re-arming must never feed a
    /// redraw-request loop through `refresh_background`.
    fn arm(&self, waker: Arc<dyn Fn() + Send + Sync>) {
        {
            let mut current = self.waker.lock().expect("layout waker");
            if current
                .as_ref()
                .is_some_and(|armed| Arc::ptr_eq(armed, &waker))
            {
                return;
            }
            *current = Some(waker);
        }
        self.pending.store(false, Ordering::Release);
        self.notify();
    }
    fn detach(&self) {
        *self.waker.lock().expect("layout waker") = None;
        self.pending.store(false, Ordering::Release);
    }
}

pub(super) struct Background {
    jobs: SyncSender<Job>,
    results: Receiver<ResultOwner>,
    epoch: Arc<AtomicU64>,
    wake: Arc<Wake>,
    active: bool,
    pending: HashSet<FlowId>,
    outstanding: usize,
    /// Last visited flow, including spacers and rejected owners.
    cursor: Option<FlowId>,
    /// Detects a changed viewport without walking a cached prefix.
    origin: Option<FlowId>,
    backwards: bool,
    exhausted: bool,
    scene_slots: usize,
    /// Width and style generation the sweep is walking for.
    layout_key: Option<(u32, u64)>,
    #[cfg(test)]
    test_worker: Option<(Receiver<Job>, SyncSender<ResultOwner>)>,
    pub(super) skipped_oversize: usize,
    pub(super) failed: bool,
}

impl Background {
    pub(super) fn start(metrics: Arc<dyn TextMetrics>, waker: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (jobs, inbox) = mpsc::sync_channel::<Job>(CAPACITY);
        let (outbox, results) = mpsc::sync_channel::<ResultOwner>(CAPACITY);
        let epoch = Arc::new(AtomicU64::new(0));
        let wake = Arc::new(Wake::default());
        let current = Arc::clone(&epoch);
        let worker_wake = Arc::clone(&wake);
        std::thread::Builder::new()
            .name("misa-layout".into())
            .spawn(move || {
                // Even a panicking TextMetrics implementation must not retry.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    while let Ok(job) = inbox.recv() {
                        let result = step(job, &current, &metrics);
                        if outbox.send(result).is_err() {
                            break;
                        }
                        worker_wake.notify();
                    }
                }));
                // Also wake when the channel disconnects without a result.
                worker_wake.notify();
            })
            .expect("layout worker thread");
        let background = Self {
            jobs,
            results,
            epoch,
            wake,
            active: true,
            pending: HashSet::new(),
            outstanding: 0,
            cursor: None,
            origin: None,
            backwards: false,
            exhausted: false,
            scene_slots: 0,
            layout_key: None,
            #[cfg(test)]
            test_worker: None,
            skipped_oversize: 0,
            failed: false,
        };
        background.wake.arm(waker);
        background
    }

    /// Resume or replace the host wake and admit work again.
    pub(super) fn resume(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.active = true;
        self.wake.arm(waker);
    }

    /// Stop admitting or waking. Work already in flight drains silently and is
    /// validated like any other result; no host callback outlives this call
    /// except one already running, which can only request a stale poll.
    pub(super) fn pause(&mut self) {
        self.active = false;
        self.wake.detach();
    }

    /// Content changed: results measured against it are now stale. The sweep
    /// cursor survives a partial edit, so typing cannot restart a
    /// whole-document walk; a full reset replaces reading order and restarts.
    pub(super) fn cancel(&mut self, restart: bool) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.pending.clear();
        self.exhausted = false;
        if restart {
            self.cursor = None;
            self.origin = None;
        }
    }

    /// Whether another poll can do work, or the worker still owes a result.
    pub(super) fn work_pending(&self) -> bool {
        self.active && !self.failed && (self.outstanding > 0 || !self.exhausted)
    }

    /// Drop the wake and let the worker drain; no thread is joined on the UI.
    pub(super) fn retire(&mut self) {
        self.pause();
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        self.retire();
    }
}

/// Advance bounded prewarm work on the UI thread. Installing heights or display
/// lists never changes the current frame: the cursor walks away from it.
pub(super) fn poll(background: &mut Background, ui: &mut DocumentUi) -> usize {
    if !background.active || background.failed {
        return 0;
    }
    // A notification is acknowledged before draining: a concurrent result after
    // this point produces a new wake, never a lost one.
    background.wake.pending.store(false, Ordering::Release);
    let epoch = background.epoch.load(Ordering::Acquire);
    let theme = ui.theme();
    let mut installed = 0;
    for _ in 0..CAPACITY {
        match background.results.try_recv() {
            Ok(result) => {
                background.outstanding -= 1;
                if result.epoch == epoch {
                    background.pending.remove(&result.id);
                }
                if result.epoch != epoch
                    || result.style_generation != ui.viewport.constraints().style_generation
                    || result.width != ui.viewport.constraints().width
                    || !ui.document.contains_flow(&result.id, &theme)
                {
                    continue;
                }
                if ui.viewport.install_measurement(
                    result.id.clone(),
                    result.width,
                    result.style_generation,
                    result.height,
                ) {
                    installed += 1;
                    if let Some(retained) = result.retained {
                        ui.retained.install(retained);
                    }
                }
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                background.failed = true;
                background.pending.clear();
                return installed;
            }
        }
    }
    let constraints = ui.viewport.constraints();
    let width = constraints.width;
    if width <= 0.0 {
        background.exhausted = true;
        return installed;
    }
    // A new layout key cleared every height with it: this is a new sweep.
    let layout_key = (width.to_bits(), constraints.style_generation);
    if background.layout_key != Some(layout_key) {
        background.layout_key = Some(layout_key);
        background.cursor = None;
        background.origin = None;
        background.exhausted = false;
    }
    let backwards = matches!(ui.viewport.position, FlowPosition::FollowTail);
    let origin = if backwards {
        ui.viewport.visible().first()
    } else {
        ui.viewport.visible().last()
    }
    .map(|placement| placement.id.clone());
    let Some(origin) = origin else {
        background.exhausted = true;
        return installed;
    };
    if background.origin.as_ref() != Some(&origin) || background.backwards != backwards {
        background.cursor = Some(origin.clone());
        background.origin = Some(origin.clone());
        background.backwards = backwards;
        background.exhausted = false;
        background.scene_slots = SCENE_PREWARM;
    }
    // An edit can remove the cursor's owner; restart from the visible origin.
    if let Some(cursor) = &background.cursor
        && !ui.document.contains_flow(cursor, &theme)
    {
        background.cursor = Some(origin);
    }
    let mut steps = 0;
    let mut attempts = 0;
    let mut capture_budget = CAPTURE_BUDGET;
    while background.outstanding < CAPACITY
        && steps < SKIP_BUDGET
        && attempts < WALK_BUDGET
        && !background.exhausted
    {
        let cursor = background.cursor.clone().expect("cursor starts at origin");
        let next = if backwards {
            ui.document.previous_flow(&cursor, &theme)
        } else {
            ui.document.next_flow(&cursor, &theme)
        };
        let Some(id) = next else {
            background.exhausted = true;
            break;
        };
        steps += 1;
        background.cursor = Some(id.clone());
        if !matches!(
            id,
            FlowId::Node(_) | FlowId::Row(_, _) | FlowId::End(_) | FlowId::Stream(_)
        ) {
            continue;
        }
        if background.pending.contains(&id) || ui.viewport.measured_height(&id).is_some() {
            continue;
        }
        attempts += 1;
        let snapshot = match OwnerSnapshot::capture(ui, &id, width, &mut capture_budget) {
            Ok(snapshot) => snapshot,
            Err(SnapshotError::Oversize) => {
                background.skipped_oversize += 1;
                continue;
            }
            Err(SnapshotError::Budget) => {
                // Retry this owner on a later poll, not its successor.
                background.cursor = Some(cursor);
                break;
            }
            Err(_) => continue,
        };
        let keep_scene = background.scene_slots > 0;
        if keep_scene {
            background.scene_slots -= 1;
        }
        let job = Job {
            id: id.clone(),
            epoch,
            width,
            style_generation: constraints.style_generation,
            keep_scene,
            snapshot,
        };
        match background.jobs.try_send(job) {
            Ok(()) => {
                background.pending.insert(id);
                background.outstanding += 1;
            }
            Err(TrySendError::Full(_)) => {
                // Retry this owner on the next poll, not its successor.
                background.cursor = Some(cursor);
                break;
            }
            Err(TrySendError::Disconnected(_)) => {
                background.failed = true;
                break;
            }
        }
    }
    // A sweep that spent its step budget continues on the next poll: one wake
    // per slice, never one per skipped owner. Once exhausted, no wake at all.
    if steps == SKIP_BUDGET && background.outstanding == 0 && !background.exhausted {
        background.wake.notify();
    }
    installed
}

#[cfg(test)]
impl Background {
    /// A background with its worker under test control: no thread, no races.
    pub(super) fn deterministic() -> Self {
        let (jobs, inbox) = mpsc::sync_channel(CAPACITY);
        let (outbox, results) = mpsc::sync_channel(CAPACITY);
        Self {
            jobs,
            results,
            epoch: Arc::new(AtomicU64::new(0)),
            wake: Arc::new(Wake::default()),
            active: true,
            pending: HashSet::new(),
            outstanding: 0,
            cursor: None,
            origin: None,
            backwards: false,
            exhausted: false,
            scene_slots: 0,
            layout_key: None,
            test_worker: Some((inbox, outbox)),
            skipped_oversize: 0,
            failed: false,
        }
    }
    /// Execute exactly one queued job synchronously, with no sleeps or races.
    pub(super) fn step_for_test(&self, metrics: Arc<dyn TextMetrics>) -> bool {
        let (inbox, outbox) = self.test_worker.as_ref().unwrap();
        let Ok(job) = inbox.try_recv() else {
            return false;
        };
        outbox.try_send(step(job, &self.epoch, &metrics)).unwrap();
        self.wake.notify();
        true
    }
    pub(super) fn cursor(&self) -> Option<&FlowId> {
        self.cursor.as_ref()
    }
    pub(super) fn pending(&self) -> usize {
        self.pending.len()
    }
    pub(super) fn outstanding(&self) -> usize {
        self.outstanding
    }
}
