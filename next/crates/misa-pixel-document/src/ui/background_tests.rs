//! Deterministic prewarm behavior: one test-owned worker step at a time, no
//! sleeps, no threads and no GPU. Whatever the worker installs must leave the
//! painted frame untouched.
use super::super::DocumentUi;
use super::super::flow::FlowId;
use super::super::tests as ui_tests;
use super::*;
use misa_proto::view::{Node, Span};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn messages(count: usize) -> Node {
    Node::section("session")
        .id("session")
        .children((0..count).map(|index| {
            Node::text("message.user", [Span::plain("a measured message body")])
                .id(format!("msg.{index}"))
        }))
}

fn app(count: usize, width: u32, height: u32) -> DocumentUi {
    let mut ui = DocumentUi::new(messages(count), ui_tests::test_metrics());
    ui.frame_at(width, height, Duration::ZERO);
    ui.background = Some(Background::deterministic());
    ui
}

/// Enqueue bounded work, run every queued job, install every result.
fn pump(ui: &mut DocumentUi, rounds: usize) {
    for _ in 0..rounds {
        ui.poll_background();
        while ui
            .background
            .as_ref()
            .expect("deterministic worker")
            .step_for_test(ui_tests::test_metrics())
        {}
        ui.poll_background();
    }
}

fn msg_index(id: &FlowId) -> usize {
    match id {
        FlowId::Node(id) => id
            .strip_prefix("msg.")
            .expect("message id")
            .parse()
            .expect("numeric message index"),
        other => panic!("unexpected flow id {other:?}"),
    }
}

fn cursor_index(ui: &DocumentUi) -> usize {
    msg_index(
        ui.background
            .as_ref()
            .expect("deterministic worker")
            .cursor()
            .expect("cursor starts at the visible origin"),
    )
}

fn first_visible_index(ui: &DocumentUi) -> usize {
    msg_index(&ui.viewport.visible().first().expect("visible owner").id)
}

#[test]
fn tail_following_prewarms_history_backwards() {
    let mut ui = app(40, 600, 200);
    assert!(matches!(
        ui.viewport.position,
        misa_pixel_ui::FlowPosition::FollowTail
    ));
    pump(&mut ui, 6);
    let first = first_visible_index(&ui);
    assert!(first > 0, "history must sit above the visible tail");
    // The cursor walks up from the visible window, never down past the tail.
    assert!(cursor_index(&ui) < first);
    // Owners just above the window carry exact heights before they are seen.
    let above = FlowId::Node(format!("msg.{}", first - 1));
    assert!(ui.viewport.measured_height(&above).is_some());
}

#[test]
fn anchored_reading_prewarms_forwards_from_the_last_visible_owner() {
    let mut ui = app(40, 600, 200);
    ui.pin_to_top();
    ui.frame_at(600, 200, Duration::ZERO);
    pump(&mut ui, 4);
    let last = msg_index(&ui.viewport.visible().last().expect("visible owner").id);
    assert!(cursor_index(&ui) > last);
}

#[test]
fn bounded_polls_progress_across_thousands_of_owners() {
    let mut ui = app(10_000, 600, 120);
    let baseline = ui.viewport.measured_count();
    // Every round admits at most CAPACITY owners; no poll scans the document.
    pump(&mut ui, 25);
    let measured = ui.viewport.measured_count() - baseline;
    assert!(
        measured > 2 * WALK_BUDGET,
        "progress must continue past one walk slice: {measured}"
    );
    assert!(
        measured <= 25 * CAPACITY,
        "one round admits at most CAPACITY owners: {measured}"
    );
    // A single poll walks at most SKIP_BUDGET flows from its persistent cursor.
    let before = cursor_index(&ui);
    ui.poll_background();
    assert!(
        before.saturating_sub(cursor_index(&ui)) <= SKIP_BUDGET,
        "one poll walks a bounded slice"
    );
    // A paused worker admits nothing at all.
    ui.pause_background();
    assert_eq!(ui.poll_background(), 0);
}

#[test]
fn adjacent_owners_keep_scenes_and_distant_owners_keep_only_heights() {
    let mut ui = app(60, 600, 120);
    pump(&mut ui, 35);
    let first = first_visible_index(&ui);
    let near = FlowId::Node(format!("msg.{}", first - 1));
    // The owners the reader will reach next are already laid out.
    assert!(ui.retained.contains(&near.cache_key()));
    // Distant owners keep their exact height only; their display list is
    // bounded away instead of retained for the whole document.
    let distant = (0..first - SCENE_PREWARM)
        .rev()
        .map(|index| FlowId::Node(format!("msg.{index}")))
        .find(|id| ui.viewport.measured_height(id).is_some())
        .expect("prewarm reaches beyond the adjacent owners");
    assert!(!ui.retained.contains(&distant.cache_key()));
}

#[test]
fn stale_results_are_dropped_after_content_and_width_changes() {
    let mut ui = app(20, 600, 120);
    ui.poll_background();
    assert!(ui.background.as_ref().unwrap().outstanding() > 0);
    // An edit invalidates every measurement made against the old content.
    ui.set_view(messages(20));
    ui.frame_at(600, 120, Duration::ZERO);
    let before = ui.viewport.measured_count();
    while ui
        .background
        .as_ref()
        .unwrap()
        .step_for_test(ui_tests::test_metrics())
    {}
    ui.poll_background();
    assert_eq!(
        ui.viewport.measured_count(),
        before,
        "results measured against old content never install"
    );

    // A resize is a relayout: measurements for another width are refused.
    ui.poll_background();
    ui.frame_at(300, 120, Duration::ZERO);
    let before = ui.viewport.measured_count();
    while ui
        .background
        .as_ref()
        .unwrap()
        .step_for_test(ui_tests::test_metrics())
    {}
    ui.poll_background();
    assert_eq!(
        ui.viewport.measured_count(),
        before,
        "results measured at another width never install"
    );
}

#[test]
fn wakes_coalesce_and_a_paused_worker_stays_silent() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    let mut ui = app(10, 600, 120);
    ui.background.as_mut().unwrap().resume(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    assert_eq!(wakes.load(Ordering::SeqCst), 1, "resuming arms the host");
    ui.poll_background();
    while ui
        .background
        .as_ref()
        .unwrap()
        .step_for_test(ui_tests::test_metrics())
    {}
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        2,
        "a burst of results requests one poll"
    );
    ui.pause_background();
    assert!(!ui.background_work_pending());
    assert_eq!(ui.poll_background(), 0);
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        2,
        "a parked worker never wakes the host"
    );
}

#[test]
fn rearming_the_same_host_wake_is_a_no_op() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    let mut ui = app(4, 600, 120);
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    ui.background.as_mut().unwrap().resume(waker.clone());
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    // The host re-arms on every poll; that must never feed a redraw loop.
    for _ in 0..32 {
        ui.background.as_mut().unwrap().resume(waker.clone());
    }
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
}

#[test]
fn edits_do_not_restart_the_sweep_or_burst_wakes() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    let mut ui = app(900, 600, 120);
    ui.background.as_mut().unwrap().resume(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    // Sweep past one skip slice so the cursor is clear of the visible window.
    pump(&mut ui, 520);
    let settled = wakes.load(Ordering::SeqCst);
    for _ in 0..20 {
        ui.invalidate("msg.450");
        ui.poll_background();
    }
    assert!(
        wakes.load(Ordering::SeqCst) - settled <= 2,
        "an edit must not restart a whole-document walk: {} wakes",
        wakes.load(Ordering::SeqCst) - settled
    );
}

#[test]
fn a_full_reset_restarts_the_sweep_from_the_visible_origin() {
    let mut ui = app(40, 600, 120);
    pump(&mut ui, 30);
    // A reset replaces reading order and every measured height with it.
    ui.set_view(messages(40));
    pump(&mut ui, 3);
    let first = first_visible_index(&ui);
    let above = FlowId::Node(format!("msg.{}", first - 1));
    assert!(
        ui.viewport.measured_height(&above).is_some(),
        "prewarm resumes from the new tail after a reset"
    );
}

#[test]
fn prewarm_never_changes_what_the_frame_paints() {
    let mut warm = app(20, 600, 200);
    pump(&mut warm, 12);
    let mut cold = app(20, 600, 200);
    let painted = warm.frame_at(600, 200, Duration::ZERO);
    let baseline = cold.frame_at(600, 200, Duration::ZERO);
    // Prewarm never changes the content of a frame. The scrollbar is
    // measurement chrome — it appears once the heights cover the source — so
    // it is compared apart below.
    fn content(scene: &misa_pixel_ui::Scene) -> Vec<String> {
        scene
            .ops
            .iter()
            .filter(|op| {
                !matches!(op, misa_pixel_ui::Op::ClipRect { x, width, .. }
                    if *x >= scene.width - 9.0 && *width <= 6.0)
            })
            .map(|op| format!("{op:?}"))
            .collect()
    }
    assert_eq!(content(&painted), content(&baseline));
}

#[test]
fn the_scrollbar_appears_only_when_the_heights_cover_the_source() {
    let mut ui = app(200, 600, 200);
    // Before the sweep completes there is no honest total: no thumb is drawn.
    ui.frame_at(600, 200, Duration::ZERO);
    assert!(ui.scrollbar().is_none());
    pump(&mut ui, 200);
    ui.frame_at(600, 200, Duration::ZERO);
    let bar = ui.scrollbar().expect("heights cover the source");
    assert!(bar.thumb.height > 0.0, "{:?}", bar.scroll);
    // Dragging the thumb to the top of the track moves the reading position.
    let before = ui.viewport.visible().first().unwrap().id.clone();
    ui.scroll_drag(bar.bounds.y + 1.0);
    let after = ui.viewport.visible().first().unwrap().id.clone();
    assert_ne!(
        after, before,
        "dragging the thumb must move the reading position"
    );
}

#[test]
fn scrolling_after_prewarm_reuses_instead_of_remeasuring() {
    let mut ui = app(20, 600, 200);
    pump(&mut ui, 12);
    let measured = ui.viewport.measured_count();
    assert!(measured > 0);
    // Reading upward adopts the prewarmed heights without clearing the index.
    ui.scroll(-600.0);
    ui.frame_at(600, 200, Duration::ZERO);
    assert!(ui.viewport.measured_count() >= measured);
    pump(&mut ui, 2);
    assert!(ui.background_work_pending() || ui.viewport.measured_count() >= measured);
}
