//! Native pixel fixture. No semantic tree, protocol, or kit is constructed here.
mod dashboard;
pub use dashboard::Dashboard;

use misa_pixel_ui::{Scene, TextMetrics};
use misa_skia_vulkan::Renderer;
use misa_style::Color;
use misa_window_core::{Event, Key, Size};
use std::sync::Arc;

const BACKGROUND: Color = Color::Rgb(20, 22, 26);

/// The pixels returned by a Vulkan GPU readback, not a CPU rasterization.
pub struct Snapshot {
    pub scene: Scene,
    pub pixels: Vec<u8>,
    pub size: Size,
}

pub struct Headless {
    dashboard: Dashboard,
    metrics: Arc<dyn TextMetrics>,
    renderer: Renderer,
    size: Size,
}

impl Headless {
    /// Fails explicitly if the Vulkan ICD, device, font, or Ganesh context is unavailable.
    pub fn new(size: Size) -> Result<Self, String> {
        if size.width == 0 || size.height == 0 {
            return Err("headless size must be nonzero".into());
        }
        Ok(Self {
            dashboard: Dashboard { selected: false },
            metrics: misa_skia_paint::text_metrics()?,
            renderer: Renderer::new()
                .map_err(|error| format!("Vulkan renderer unavailable: {error}"))?,
            size,
        })
    }

    pub fn device_name(&self) -> &str {
        &self.renderer.device_name
    }

    /// Only normalized physical-pixel events are accepted. Text space is the
    /// window adapter's representation of the Space key.
    pub fn input(&mut self, event: Event) {
        match event {
            Event::Key(Key::Enter { .. }) => self.dashboard.toggle(),
            Event::Text(text) if text == " " => self.dashboard.toggle(),
            Event::Pointer {
                x,
                y,
                dragging: false,
            } => {
                self.dashboard
                    .click(x, y, self.size.width, self.metrics.as_ref());
            }
            Event::Resize(size) => self.size = size,
            _ => {}
        }
    }

    pub fn frame(&mut self) -> Result<Snapshot, String> {
        if self.size.width == 0 || self.size.height == 0 {
            return Err("cannot render a zero-sized headless frame".into());
        }
        let scene = self
            .dashboard
            .frame(self.size.width, self.size.height, self.metrics.as_ref());
        let pixels = self.renderer.render(&scene, BACKGROUND)?.into_raw();
        Ok(Snapshot {
            scene,
            pixels,
            size: self.size,
        })
    }
}

/// No display server or window host is required.
pub fn run() -> Result<(), String> {
    let mut host = Headless::new(Size {
        width: 640,
        height: 480,
    })?;
    println!("Native headless Vulkan/Ganesh: {}", host.device_name());
    let initial = host.frame()?;
    host.input(Event::Key(Key::Enter { newline: false }));
    let selected = host.frame()?;
    if initial.pixels == selected.pixels {
        return Err("key input did not alter GPU readback".into());
    }
    host.input(Event::Pointer {
        x: 40.0,
        y: 155.0,
        dragging: false,
    });
    if host.frame()?.pixels != initial.pixels {
        return Err("pointer input did not restore GPU readback".into());
    }
    host.input(Event::Resize(Size {
        width: 600,
        height: 400,
    }));
    let resized = host.frame()?;
    if resized.size
        != (Size {
            width: 600,
            height: 400,
        })
        || resized.pixels.len() != 600 * 400 * 4
    {
        return Err("resize did not reach GPU readback".into());
    }
    println!(
        "Offscreen RGBA readback: {}x{} ({} bytes)",
        resized.size.width,
        resized.size.height,
        resized.pixels.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixel(snapshot: &Snapshot, x: usize, y: usize) -> &[u8] {
        let offset = (y * snapshot.size.width as usize + x) * 4;
        &snapshot.pixels[offset..offset + 4]
    }

    #[test]
    fn native_gpu_input_and_resize_without_display() {
        let mut host = Headless::new(Size {
            width: 500,
            height: 320,
        })
        .expect("Vulkan ICD and GPU readback required; configure VK_ICD_FILENAMES");
        let initial = host.frame().unwrap();
        assert_eq!(pixel(&initial, 0, 0), [20, 22, 26, 255]);
        assert_eq!(pixel(&initial, 50, 160), [65, 74, 86, 255]);
        host.input(Event::Key(Key::Enter { newline: false }));
        let keyed = host.frame().unwrap();
        assert_eq!(pixel(&keyed, 50, 160), [45, 105, 150, 255]);
        assert_ne!(initial.pixels, keyed.pixels);
        host.input(Event::Pointer {
            x: 40.0,
            y: 155.0,
            dragging: false,
        });
        assert_eq!(initial.pixels, host.frame().unwrap().pixels);
        host.input(Event::Text(" ".into()));
        assert_eq!(keyed.pixels, host.frame().unwrap().pixels);
        host.input(Event::Resize(Size {
            width: 130,
            height: 300,
        }));
        let narrow = host.frame().unwrap();
        assert_eq!(
            narrow.size,
            Size {
                width: 130,
                height: 300
            }
        );
        assert_eq!(narrow.pixels.len(), 130 * 300 * 4);
        // The 58px-wide button ends at x=94 after resize.
        assert_eq!(pixel(&narrow, 80, 160), [45, 105, 150, 255]);
        host.input(Event::Pointer {
            x: 94.0,
            y: 155.0,
            dragging: false,
        });
        assert_eq!(narrow.pixels, host.frame().unwrap().pixels);
    }
}
