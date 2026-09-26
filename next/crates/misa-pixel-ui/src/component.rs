//! Local-coordinate composition: the same tree defines clips for paint and hit.
use crate::{Op, Rect};
use std::sync::Arc;

/// Bounds are relative to the parent. Paint ops are relative to this node.
/// Every node clips both its paint and descendants to its bounds. Last child
/// paints on top and receives pointer input first.
pub struct PlacedComponent<Id> {
    pub id: Id,
    pub bounds: Rect,
    pub ops: Arc<Vec<Op>>,
    pub children: Vec<PlacedComponent<Id>>,
}

impl<Id> PlacedComponent<Id> {
    /// Build the scene op using exactly the transforms/clips used by `hit`.
    pub fn paint(&self) -> Op {
        let mut ops = self.ops.as_ref().clone();
        ops.extend(self.children.iter().map(Self::paint));
        Op::Group {
            x: self.bounds.x,
            y: self.bounds.y,
            ops: Arc::new(vec![Op::ClipRect {
                x: 0.0,
                y: 0.0,
                width: self.bounds.width.max(0.0),
                height: self.bounds.height.max(0.0),
                ops: Arc::new(ops),
            }]),
        }
    }

    /// Pointer coordinates are local to the parent. Returns deepest hit plus
    /// pointer coordinates local to that component.
    pub fn hit(&self, x: f32, y: f32) -> Option<(&Id, f32, f32)> {
        if !self.bounds.contains(x, y) {
            return None;
        }
        let (x, y) = (x - self.bounds.x, y - self.bounds.y);
        for child in self.children.iter().rev() {
            if let Some(hit) = child.hit(x, y) {
                return Some(hit);
            }
        }
        Some((&self.id, x, y))
    }

    /// Route wheel from deepest clipped child to ancestors. The callback
    /// returns unused signed pixels; non-scrollable nodes return all of them.
    pub fn wheel(
        &self,
        x: f32,
        y: f32,
        mut delta: f32,
        consume: &mut impl FnMut(&Id, f32) -> f32,
    ) -> f32 {
        if !self.bounds.contains(x, y) {
            return delta;
        }
        let (x, y) = (x - self.bounds.x, y - self.bounds.y);
        for child in self.children.iter().rev() {
            if child.bounds.contains(x, y) {
                delta = child.wheel(x, y, delta, consume);
                break;
            }
        }
        if delta != 0.0 {
            consume(&self.id, delta)
        } else {
            delta
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(
        id: u8,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        children: Vec<PlacedComponent<u8>>,
    ) -> PlacedComponent<u8> {
        PlacedComponent {
            id,
            bounds: Rect {
                x,
                y,
                width: w,
                height: h,
            },
            ops: Arc::new(vec![]),
            children,
        }
    }
    #[test]
    fn nested_clip_hit_paint_and_wheel_bubble() {
        let tree = node(
            0,
            10.0,
            10.0,
            50.0,
            50.0,
            vec![node(
                1,
                20.0,
                20.0,
                20.0,
                40.0,
                vec![node(2, 2.0, 3.0, 20.0, 20.0, vec![])],
            )],
        );
        assert_eq!(tree.hit(33.0, 34.0), Some((&2, 1.0, 1.0)));
        assert_eq!(tree.hit(35.0, 61.0), None); // parent clip
        assert_eq!(tree.hit(31.0, 31.0), Some((&1, 1.0, 1.0)));
        let mut visited = vec![];
        let unused = tree.wheel(33.0, 34.0, 12.0, &mut |id, delta| {
            visited.push((*id, delta));
            match id {
                2 => delta - 4.0,
                1 => delta - 3.0,
                _ => 0.0,
            }
        });
        assert_eq!(unused, 0.0);
        assert_eq!(visited, vec![(2, 12.0), (1, 8.0), (0, 5.0)]);
        assert!(matches!(tree.paint(), Op::Group { x: 10.0, y: 10.0, ops }
            if matches!(&ops[0], Op::ClipRect { width: 50.0, height: 50.0, ops, .. }
                if matches!(&ops[0], Op::Group { x: 20.0, y: 20.0, .. }))));
    }
}
