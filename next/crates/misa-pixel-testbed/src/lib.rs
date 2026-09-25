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
            dashboard: Dashboard::default(),
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
            Event::Key(Key::Backspace) => self.dashboard.backspace_note(),
            Event::Key(Key::Enter { .. }) if !self.dashboard.note_focused() => {
                self.dashboard.toggle()
            }
            Event::Text(text) if !self.dashboard.note_focused() && text == " " => {
                self.dashboard.toggle()
            }
            Event::Text(text) => self.dashboard.insert_note(&text),
            Event::Pointer {
                x,
                y,
                dragging: false,
            } => {
                self.dashboard
                    .click(x, y, self.size.width, self.metrics.as_ref());
            }
            Event::Wheel { delta } => self.dashboard.scroll(delta),
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
    fn editable_note_uses_gpu_caret_clip_and_resize() {
        let mut host = Headless::new(Size {
            width: 130,
            height: 300,
        })
        .expect("Vulkan ICD and GPU readback required; configure VK_ICD_FILENAMES");
        let initial = host.frame().unwrap();
        host.input(Event::Pointer {
            x: 40.0,
            y: 50.0,
            dragging: false,
        });
        let focused = host.frame().unwrap();
        assert_ne!(pixel(&initial, 35, 50), pixel(&focused, 35, 50));
        host.input(Event::Text("éabcdefghijklmnop".into()));
        let typed = host.frame().unwrap();
        assert_eq!(host.dashboard.note(), "éabcdefghijklmnop");
        assert_ne!(focused.pixels, typed.pixels);
        // The text and scrolled caret are confined to the field's measured clip.
        let clip = typed
            .scene
            .ops
            .iter()
            .find_map(|op| match op {
                misa_pixel_ui::Op::ClipRect {
                    x: 43.0,
                    y: 50.0,
                    width,
                    ops,
                    ..
                } if *width == 44.0 => Some(ops),
                _ => None,
            })
            .expect("placed note clip");
        assert!(
            matches!(clip.last(), Some(misa_pixel_ui::Op::Rect { x, .. }) if *x >= 43.0 && *x < 87.0)
        );
        assert_eq!(pixel(&typed, 90, 56), pixel(&focused, 90, 56));
        host.input(Event::Key(Key::Backspace));
        let erased = host.frame().unwrap();
        assert_eq!(host.dashboard.note(), "éabcdefghijklmno");
        assert_ne!(typed.pixels, erased.pixels);
        host.input(Event::Resize(Size {
            width: 500,
            height: 320,
        }));
        let wide = host.frame().unwrap();
        assert_eq!(wide.pixels.len(), 500 * 320 * 4);
        assert!((43..87).any(|x| (50..65).any(|y| pixel(&erased, x, y) != pixel(&wide, x, y))));
        host.input(Event::Pointer {
            x: 0.0,
            y: 0.0,
            dragging: false,
        });
        host.input(Event::Text("ignored".into()));
        host.input(Event::Key(Key::Backspace));
        assert_eq!(host.dashboard.note(), "éabcdefghijklmno");
    }

    #[test]
    fn wheel_resize_and_follow_change_real_gpu_pixels() {
        let mut host = Headless::new(Size {
            width: 130,
            height: 300,
        })
        .expect("Vulkan ICD and GPU readback required; configure VK_ICD_FILENAMES");
        let top = host.frame().unwrap();
        assert_eq!(host.dashboard.offset(), 0.0);
        host.input(Event::Wheel { delta: 26.0 });
        let scrolled = host.frame().unwrap();
        assert_eq!(host.dashboard.offset(), 26.0);
        assert_ne!(pixel(&top, 38, 208), pixel(&scrolled, 38, 208));
        host.input(Event::Wheel { delta: 100_000.0 });
        let end = host.frame().unwrap();
        assert_eq!(host.dashboard.offset(), 20.0 * 26.0 - 35.0);
        assert_ne!(scrolled.pixels, end.pixels);
        host.input(Event::Resize(Size {
            width: 130,
            height: 800,
        }));
        let taller = host.frame().unwrap();
        assert_eq!(host.dashboard.offset(), 0.0);
        assert_eq!(pixel(&taller, 38, 208), pixel(&top, 38, 208));
        host.dashboard.follow_tail();
        host.input(Event::Resize(Size {
            width: 130,
            height: 300,
        }));
        host.frame().unwrap();
        assert_eq!(host.dashboard.offset(), 20.0 * 26.0 - 35.0);
        host.dashboard.append_row();
        let appended = host.frame().unwrap();
        assert_eq!(host.dashboard.offset(), 21.0 * 26.0 - 35.0);
        assert_ne!(end.pixels, appended.pixels);
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
