use misa_pixel_ui::flow::{Constraints, FlowPosition, FlowSource, FlowViewport};
use misa_pixel_ui::{PlacedComponent, Rect};
use std::cell::RefCell;
use std::sync::Arc;

struct Indexed {
    count: usize,
    visits: RefCell<Vec<usize>>,
}
impl Indexed {
    fn new(count: usize) -> Self {
        Self {
            count,
            visits: RefCell::new(vec![]),
        }
    }
    fn take(&self) -> Vec<usize> {
        std::mem::take(&mut self.visits.borrow_mut())
    }
}
impl FlowSource for Indexed {
    type Id = usize;
    fn first(&self) -> Option<usize> {
        (self.count > 0).then_some(0)
    }
    fn last(&self) -> Option<usize> {
        self.count.checked_sub(1)
    }
    fn previous(&self, id: &usize) -> Option<usize> {
        id.checked_sub(1)
    }
    fn next(&self, id: &usize) -> Option<usize> {
        (id + 1 < self.count).then_some(id + 1)
    }
    fn contains(&self, id: &usize) -> bool {
        *id < self.count
    }
    fn measure(&mut self, id: &usize, width: f32, _: u64) -> f32 {
        self.visits.borrow_mut().push(*id);
        if width < 50.0 { 25.0 } else { 20.0 }
    }
}
fn size(width: f32, height: f32) -> Constraints {
    Constraints {
        width,
        height,
        style_generation: 0,
    }
}
#[test]
fn indexed_tail_jump_wheel_and_resize_are_sparse_and_exact() {
    let mut src = Indexed::new(100_000);
    let mut view = FlowViewport::default();
    view.layout(&mut src, size(100.0, 45.0));
    assert_eq!(src.take(), vec![99_999, 99_998, 99_997]);
    assert_eq!(
        view.visible()
            .iter()
            .map(|p| (p.id, p.y))
            .collect::<Vec<_>>(),
        vec![(99_997, -15.0), (99_998, 5.0), (99_999, 25.0)]
    );
    view.layout(&mut src, size(100.0, 45.0));
    assert!(src.take().is_empty());
    view.anchor(50_000, 5.0, 10.0);
    view.layout(&mut src, size(100.0, 45.0));
    assert_eq!(src.take(), vec![50_000, 50_001, 49_999]);
    assert_eq!(view.visible()[1].id, 50_000);
    assert_eq!(view.wheel(&mut src, -20.0), 0.0);
    assert_eq!(src.take(), vec![49_998]);
    let anchor = view.visible().iter().find(|p| p.id == 50_000).unwrap().y;
    view.anchor(50_000, 0.0, anchor);
    view.layout(&mut src, size(40.0, 45.0));
    assert_eq!(
        view.visible().iter().find(|p| p.id == 50_000).unwrap().y,
        anchor
    );
    assert!(src.take().len() <= 5);
    view.invalidate(&50_000);
    view.layout(&mut src, size(40.0, 45.0));
    assert_eq!(src.take(), vec![50_000]);
    view.layout(
        &mut src,
        Constraints {
            style_generation: 1,
            ..size(40.0, 45.0)
        },
    );
    assert!(src.take().len() <= 5);
}
#[test]
fn zero_height_structural_owner_does_not_change_exact_positions() {
    struct Source;
    impl FlowSource for Source {
        type Id = usize;
        fn first(&self) -> Option<usize> {
            Some(0)
        }
        fn last(&self) -> Option<usize> {
            Some(2)
        }
        fn previous(&self, id: &usize) -> Option<usize> {
            id.checked_sub(1)
        }
        fn next(&self, id: &usize) -> Option<usize> {
            (*id < 2).then_some(id + 1)
        }
        fn contains(&self, id: &usize) -> bool {
            *id <= 2
        }
        fn measure(&mut self, id: &usize, _: f32, _: u64) -> f32 {
            if *id == 1 { 0.0 } else { 20.0 }
        }
    }
    let mut view = FlowViewport::default();
    view.layout(&mut Source, size(100.0, 30.0));
    assert_eq!(
        view.visible()
            .iter()
            .map(|item| (item.id, item.y))
            .collect::<Vec<_>>(),
        vec![(0, -10.0), (2, 10.0)]
    );
    assert_eq!(view.wheel(&mut Source, -10.0), 0.0);
    assert_eq!(
        view.visible()
            .iter()
            .map(|item| (item.id, item.y))
            .collect::<Vec<_>>(),
        vec![(0, 0.0), (2, 20.0)]
    );
}

#[test]
fn short_content_edges_and_anchor_survive_edits() {
    let mut src = Indexed::new(2);
    let mut view = FlowViewport::default();
    view.layout(&mut src, size(100.0, 100.0));
    assert_eq!(
        view.visible().iter().map(|p| p.y).collect::<Vec<_>>(),
        vec![0.0, 20.0]
    );
    assert_eq!(view.wheel(&mut src, -30.0), -30.0);
    assert_eq!(view.wheel(&mut src, 30.0), 30.0);
    view.pin_top(&src);
    view.layout(&mut src, size(100.0, 100.0));
    assert!(matches!(
        view.position,
        FlowPosition::Anchor {
            id: 0,
            screen_y: 0.0,
            ..
        }
    ));
    view.anchor(1, 4.0, 25.0);
    view.layout(&mut src, size(100.0, 25.0));
    assert_eq!(view.visible()[1].y, 20.0); // clamped at head
    let mut reduced = Indexed::new(1);
    view.remove(&reduced, &1);
    view.layout(&mut reduced, size(100.0, 25.0));
    assert_eq!(view.visible()[0].id, 0);
    view.replace(&0, 0);
    view.layout(&mut reduced, size(100.0, 25.0));
    assert_eq!(reduced.take(), vec![0]);
    view.follow_tail();
    let mut extended = Indexed::new(5);
    view.layout(&mut extended, size(100.0, 25.0));
    assert!(matches!(view.position, FlowPosition::FollowTail));
    assert_eq!(view.visible().last().unwrap().id, 4);
    assert_eq!(view.visible().last().unwrap().y, 5.0);
    assert_eq!(view.constraints().height, 25.0);
}

#[test]
fn stable_id_replacement_and_removal_keep_nearest_screen_anchor() {
    struct Sparse(Vec<u32>);
    impl FlowSource for Sparse {
        type Id = u32;
        fn first(&self) -> Option<u32> {
            self.0.first().copied()
        }
        fn last(&self) -> Option<u32> {
            self.0.last().copied()
        }
        fn previous(&self, id: &u32) -> Option<u32> {
            self.0
                .iter()
                .position(|i| i == id)
                .and_then(|n| n.checked_sub(1))
                .map(|n| self.0[n])
        }
        fn next(&self, id: &u32) -> Option<u32> {
            self.0
                .iter()
                .position(|i| i == id)
                .and_then(|n| self.0.get(n + 1).copied())
        }
        fn contains(&self, id: &u32) -> bool {
            self.0.contains(id)
        }
        fn measure(&mut self, _: &u32, _: f32, _: u64) -> f32 {
            20.0
        }
    }
    let mut view = FlowViewport::default();
    view.anchor(20, 4.0, 20.0);
    view.layout(&mut Sparse(vec![10, 20, 30, 40]), size(100.0, 30.0));
    assert_eq!(view.visible().iter().find(|p| p.id == 20).unwrap().y, 16.0);
    view.replace(&20, 25);
    view.layout(&mut Sparse(vec![10, 25, 30, 40]), size(100.0, 30.0));
    assert_eq!(view.visible().iter().find(|p| p.id == 25).unwrap().y, 16.0);
    assert!(matches!(
        view.position,
        FlowPosition::Anchor {
            id: 25,
            local_y: 4.0,
            screen_y: 20.0
        }
    ));
    let mut removed = Sparse(vec![10, 30, 40]);
    view.remove(&removed, &25);
    view.layout(&mut removed, size(100.0, 30.0));
    assert!(matches!(view.position, FlowPosition::Anchor { id: 10, .. }));
    assert_eq!(view.visible().iter().find(|p| p.id == 10).unwrap().y, -4.0);
}

#[test]
fn wheel_matches_exact_clamped_offsets_across_both_edges() {
    for count in 0..8 {
        let mut source = Indexed::new(count);
        let mut view = FlowViewport::default();
        view.layout(&mut source, size(100.0, 45.0));
        let mut offset = (count as f32 * 20.0 - 45.0).max(0.0);
        for delta in [-300.0, 7.0, 13.0, 55.0, 300.0, -21.0, -300.0] {
            let wanted = (offset + delta).clamp(0.0, (count as f32 * 20.0 - 45.0).max(0.0));
            let unused = view.wheel(&mut source, delta);
            assert!(
                (unused - (delta - (wanted - offset))).abs() < 0.001,
                "count={count} delta={delta}"
            );
            offset = wanted;
            let expected = (0..count)
                .filter_map(|id| {
                    let y = id as f32 * 20.0 - offset;
                    (y < 45.0 && y + 20.0 > 0.0).then_some((id, y))
                })
                .collect::<Vec<_>>();
            assert_eq!(
                view.visible()
                    .iter()
                    .map(|p| (p.id, p.y))
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[test]
fn bounded_nested_flow_uses_child_clip_and_bubbles_unused_wheel() {
    let mut outer_source = Indexed::new(20);
    let mut child_source = Indexed::new(2);
    let mut outer = FlowViewport::default();
    let mut child = FlowViewport::default();
    outer.pin_top(&outer_source);
    outer.layout(&mut outer_source, size(100.0, 60.0));
    child.layout(&mut child_source, size(40.0, 30.0));
    // Child content is 40 high but the owner allocated by the parent remains
    // the bounded 30-pixel container, regardless of child's scroll state.
    let tree = PlacedComponent {
        id: 0,
        bounds: Rect {
            x: 10.0,
            y: 10.0,
            width: 100.0,
            height: 60.0,
        },
        ops: Arc::new(vec![]),
        children: vec![PlacedComponent {
            id: 1,
            bounds: Rect {
                x: 5.0,
                y: 5.0,
                width: 40.0,
                height: 30.0,
            },
            ops: Arc::new(vec![]),
            children: vec![],
        }],
    };
    assert_eq!(tree.hit(20.0, 44.0), Some((&1, 5.0, 29.0)));
    assert_eq!(tree.hit(20.0, 45.0), Some((&0, 10.0, 35.0)));
    assert_eq!(tree.hit(20.0, 70.0), None);
    let mut order = vec![];
    let unused = tree.wheel(20.0, 20.0, 35.0, &mut |id, delta| {
        order.push(*id);
        match id {
            1 => child.wheel(&mut child_source, delta),
            _ => outer.wheel(&mut outer_source, delta),
        }
    });
    assert_eq!(order, vec![1, 0]);
    assert_eq!(unused, 0.0);
    assert_eq!(child.visible().last().unwrap().id, 1);
    assert_eq!(outer.visible().first().unwrap().id, 1);
    assert_eq!(outer.constraints().height, 60.0);
}
