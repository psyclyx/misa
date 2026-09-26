//! Standalone native GPU smoke example for nested, exact measured flows.
use misa_pixel_ui::{FlowConstraints, FlowSource, FlowViewport, Op, PlacedComponent, Rect, Scene};
use misa_skia_vulkan::Renderer;
use misa_style::{Color, Style};
use std::sync::Arc;

struct Rows {
    count: usize,
    height: f32,
}
impl FlowSource for Rows {
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
    fn measure(&mut self, id: &usize, _: f32, _: u64) -> f32 {
        if self.count == 20 && *id == 1 {
            60.0
        } else {
            self.height
        }
    }
}

fn paint_rows(view: &FlowViewport<usize>, width: f32, tint: (u8, u8, u8)) -> Arc<Vec<Op>> {
    Arc::new(
        view.visible()
            .iter()
            .map(|row| Op::Rect {
                x: 0.0,
                y: row.y,
                width,
                height: row.height - 2.0,
                style: Style::rgb(tint.0.wrapping_add((row.id % 2) as u8 * 25), tint.1, tint.2),
            })
            .collect(),
    )
}

fn tree(outer: &FlowViewport<usize>, child: &FlowViewport<usize>) -> PlacedComponent<u8> {
    let child_y = outer
        .visible()
        .iter()
        .find(|row| row.id == 1)
        .map_or(-100.0, |row| row.y);
    PlacedComponent {
        id: 0,
        bounds: Rect {
            x: 10.0,
            y: 10.0,
            width: 180.0,
            height: 120.0,
        },
        ops: paint_rows(outer, 180.0, (34, 65, 95)),
        children: vec![PlacedComponent {
            id: 1,
            bounds: Rect {
                x: 18.0,
                y: child_y,
                width: 100.0,
                height: 60.0,
            },
            ops: paint_rows(child, 100.0, (135, 70, 40)),
            children: vec![],
        }],
    }
}

/// Run with `misa-pixel-testbed --nested-flow`; no window, protocol or client.
pub fn run() -> Result<(), String> {
    let mut renderer = Renderer::new().map_err(|e| format!("Vulkan renderer unavailable: {e}"))?;
    let mut outer_source = Rows {
        count: 20,
        height: 24.0,
    };
    let mut child_source = Rows {
        count: 8,
        height: 20.0,
    };
    let mut outer = FlowViewport::default();
    let mut child = FlowViewport::default();
    outer.pin_top(&outer_source);
    child.pin_top(&child_source);
    outer.layout(
        &mut outer_source,
        FlowConstraints {
            width: 180.0,
            height: 120.0,
            style_generation: 0,
        },
    );
    child.layout(
        &mut child_source,
        FlowConstraints {
            width: 100.0,
            height: 60.0,
            style_generation: 0,
        },
    );
    let mut render = |root: PlacedComponent<u8>| {
        renderer
            .render(
                &Scene {
                    width: 200.0,
                    height: 140.0,
                    ops: vec![root.paint()],
                },
                Color::Rgb(20, 22, 26),
            )
            .map(|image| image.into_raw())
    };
    let before = render(tree(&outer, &child))?;
    let unused = tree(&outer, &child).wheel(35.0, 40.0, 130.0, &mut |id, delta| {
        if *id == 1 {
            child.wheel(&mut child_source, delta)
        } else {
            outer.wheel(&mut outer_source, delta)
        }
    });
    let after = render(tree(&outer, &child))?;
    if unused != 0.0 || before == after {
        return Err("nested flow wheel did not change native GPU readback".into());
    }
    println!("Nested bounded flow: child consumed wheel before parent; GPU readback changed");
    Ok(())
}
