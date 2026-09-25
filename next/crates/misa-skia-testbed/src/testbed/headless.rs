//! The same fixture/input driver as the native window, with a synthetic clock
//! and a real offscreen Ganesh/Vulkan render target. No winit event loop, display
//! handle, swapchain, transport, or raster fallback is constructed here.
use super::Fixtures;
use crate::app::Control;
use misa_skia_vulkan::Renderer;
use misa_style::Color;
use misa_window_core::{Clock, Event, Key, Size};
use std::time::Duration;

const BACKGROUND: Color = Color::Rgb(20, 22, 26);

/// A scene and the RGBA pixels read back from the GPU at the same fake instant.
pub struct Snapshot {
    pub scene: crate::Scene,
    pub pixels: image::RgbaImage,
    pub deadline: Option<Duration>,
}

#[derive(Default)]
struct FakeClock(Duration);
impl Clock for FakeClock {
    fn elapsed(&self) -> Duration {
        self.0
    }
}

pub struct Headless {
    fixtures: Fixtures,
    renderer: Renderer,
    size: Size,
    clock: FakeClock,
}

impl Headless {
    /// Fails if the Vulkan loader, device, or Ganesh context is unavailable.
    pub fn new(size: Size) -> Result<Self, String> {
        if size.width == 0 || size.height == 0 {
            return Err("headless size must be nonzero".into());
        }
        Ok(Self {
            fixtures: Fixtures::new()?,
            renderer: Renderer::new()?,
            size,
            clock: FakeClock::default(),
        })
    }

    pub fn device_name(&self) -> &str {
        &self.renderer.device_name
    }

    pub fn select_semantic(&mut self) {
        self.fixtures.select("2");
    }

    pub fn select_native(&mut self) {
        self.fixtures.select("1");
    }

    /// Center of the visible fixture field, for pointer-based test scripts.
    /// Call after a semantic frame has populated the App hit map.
    pub fn field_hit(&self) -> Option<(f32, f32)> {
        self.fixtures.semantic.control_center(&Control::Field {
            node: "panel.input".into(),
            field: "note".into(),
        })
    }

    /// Advance elapsed time since creation, never using wall time or sleeps.
    pub fn advance_to(&mut self, elapsed: Duration) {
        assert!(
            elapsed >= self.clock.elapsed(),
            "fake clock cannot run backwards"
        );
        self.clock.0 = elapsed;
    }

    /// Use exactly the backend-neutral events delivered by the window host.
    pub fn input(&mut self, event: Event) {
        if let Event::Resize(size) = &event {
            self.size = *size;
        }
        self.fixtures
            .input(event, self.clock.elapsed(), self.size.width);
    }

    /// A redraw drives App::drive(Event::Redraw), which calls App::frame_at,
    /// then renders that same scene through the Vulkan offscreen readback path.
    pub fn frame(&mut self) -> Result<Snapshot, String> {
        if self.size.width == 0 || self.size.height == 0 {
            return Err("cannot render a zero-sized headless frame".into());
        }
        let scene = self.fixtures.frame_at(
            self.size.width,
            self.size.height,
            Some(self.clock.elapsed()),
        );
        let pixels = self.renderer.render(&scene, BACKGROUND)?;
        Ok(Snapshot {
            scene,
            pixels,
            deadline: self.fixtures.deadline,
        })
    }
}

/// Built-in smoke sequence, intentionally not a benchmark or a CPU substitute.
pub fn run() -> Result<(), String> {
    let mut host = Headless::new(Size {
        width: 640,
        height: 480,
    })?;
    println!("Headless Vulkan/Ganesh: {}", host.device_name());
    let native = host.frame()?;
    host.input(Event::Key(Key::Enter { newline: false }));
    let selected = host.frame()?;
    if native.pixels == selected.pixels {
        return Err("native key input did not alter GPU readback".into());
    }
    host.input(Event::Pointer {
        x: 40.0,
        y: 155.0,
        dragging: false,
    });
    if host.frame()?.pixels != native.pixels {
        return Err("native pointer input did not restore GPU readback".into());
    }
    host.select_semantic();
    let first = host.frame()?;
    host.input(Event::Text("!".into()));
    let edited = host.frame()?;
    if first.pixels == edited.pixels {
        return Err("semantic text input did not alter GPU readback".into());
    }
    host.advance_to(Duration::from_millis(320));
    if host.frame()?.pixels == edited.pixels {
        return Err("semantic clock did not alter GPU readback".into());
    }
    host.input(Event::Resize(Size {
        width: 600,
        height: 400,
    }));
    let changed = host.frame()?;
    if changed.pixels.dimensions() != (600, 400) {
        return Err("semantic resize did not reach GPU readback".into());
    }
    println!(
        "Offscreen RGBA readback: {}x{} ({} bytes), next pulse: {:?}",
        changed.pixels.width(),
        changed.pixels.height(),
        changed.pixels.as_raw().len(),
        changed.deadline
    );
    Ok(())
}
