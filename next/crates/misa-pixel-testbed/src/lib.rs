//! Native pixel fixture. No semantic tree, protocol, or kit is constructed here.
mod dashboard;
pub use dashboard::Dashboard;
pub mod flow_demo;

use misa_pixel_ui::{ListKey, Scene, TextMetrics};
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
            Event::Key(Key::Enter { newline: true }) if self.dashboard.note_focused() => {
                self.dashboard.insert_note("\n")
            }
            Event::Key(Key::Enter { .. }) => self.dashboard.key(
                ListKey::Enter,
                self.size.width,
                self.size.height,
                self.metrics.as_ref(),
            ),
            Event::Key(Key::Up) => self.dashboard.key(
                ListKey::Up,
                self.size.width,
                self.size.height,
                self.metrics.as_ref(),
            ),
            Event::Key(Key::Down) => self.dashboard.key(
                ListKey::Down,
                self.size.width,
                self.size.height,
                self.metrics.as_ref(),
            ),
            Event::Key(Key::Home) => self.dashboard.key(
                ListKey::Home,
                self.size.width,
                self.size.height,
                self.metrics.as_ref(),
            ),
            Event::Key(Key::End) => self.dashboard.key(
                ListKey::End,
                self.size.width,
                self.size.height,
                self.metrics.as_ref(),
            ),
            Event::Key(Key::Tab { backward }) => self.dashboard.tab(backward),
            Event::Text(text) if !self.dashboard.note_focused() && text == " " => {
                self.dashboard.space()
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
    host.input(Event::Pointer {
        x: 40.0,
        y: 155.0,
        dragging: false,
    });
    let selected = host.frame()?;
    if initial.pixels == selected.pixels {
        return Err("pointer input did not alter GPU readback".into());
    }
    host.input(Event::Text(" ".into()));
    if host.frame()?.pixels != initial.pixels {
        return Err("keyboard activation did not restore GPU readback".into());
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
    // The Vulkan/Ganesh device is shared by the process; don't race readbacks.
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn pixel(snapshot: &Snapshot, x: usize, y: usize) -> &[u8] {
        let offset = (y * snapshot.size.width as usize + x) * 4;
        &snapshot.pixels[offset..offset + 4]
    }

    #[test]
    fn editable_note_uses_gpu_caret_clip_and_resize() {
        let _gpu = GPU.lock().unwrap();
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
    fn list_keyboard_wheel_pointer_and_progress_gpu_readback() {
        let _gpu = GPU.lock().unwrap();
        let mut host = Headless::new(Size {
            width: 500,
            height: 500,
        })
        .expect("Vulkan ICD and GPU readback required; configure VK_ICD_FILENAMES");
        let top = host.frame().unwrap();
        let list_top = match top.scene.ops.last().unwrap() {
            misa_pixel_ui::Op::ClipRect { y, .. } => *y as usize,
            _ => panic!("list clip missing"),
        };
        host.input(Event::Pointer {
            x: 40.0,
            y: list_top as f32 + 5.0,
            dragging: false,
        });
        let selected = host.frame().unwrap();
        assert_eq!(host.dashboard.list_selection(), Some(0));
        assert_ne!(pixel(&top, 40, 197), pixel(&selected, 40, 197));
        host.input(Event::Key(Key::End));
        let end = host.frame().unwrap();
        assert!(host.dashboard.offset() > 0.0);
        assert_ne!(
            pixel(&selected, 40, list_top + 5),
            pixel(&end, 40, list_top + 5)
        );
        host.input(Event::Key(Key::Enter { newline: false }));
        let committed = host.frame().unwrap();
        assert_eq!(host.dashboard.list_selection(), Some(19));
        assert_ne!(pixel(&selected, 80, 197), pixel(&committed, 80, 197));
        host.input(Event::Key(Key::Home));
        host.frame().unwrap();
        assert_eq!(host.dashboard.offset(), 0.0);
        host.input(Event::Wheel { delta: 26.0 });
        let wheel = host.frame().unwrap();
        assert_eq!(host.dashboard.offset(), 26.0);
        assert_ne!(top.pixels, wheel.pixels);
        host.input(Event::Resize(Size {
            width: 130,
            height: 300,
        }));
        let narrow = host.frame().unwrap();
        assert_eq!(narrow.pixels.len(), 130 * 300 * 4);
        assert_eq!(host.dashboard.offset(), 26.0); // retained offset stays clamped
        assert!(matches!(
            narrow.scene.ops.last(),
            Some(misa_pixel_ui::Op::ClipRect {
                width: 58.0,
                height: 0.0,
                ..
            })
        ));
    }

    #[test]
    fn checkbox_focus_space_hit_and_narrow_clip_on_gpu() {
        let _gpu = GPU.lock().unwrap();
        let mut host = Headless::new(Size {
            width: 500,
            height: 500,
        })
        .expect("Vulkan ICD and GPU readback required; configure VK_ICD_FILENAMES");
        let initial = host.frame().unwrap();
        host.input(Event::Key(Key::Tab { backward: false })); // note
        host.input(Event::Key(Key::Tab { backward: false })); // checkbox
        let focused = host.frame().unwrap();
        assert_ne!(pixel(&initial, 37, 128), pixel(&focused, 37, 128));
        host.input(Event::Text(" ".into()));
        let checked = host.frame().unwrap();
        assert!(host.dashboard.checked());
        assert_ne!(pixel(&focused, 47, 135), pixel(&checked, 47, 135));
        host.input(Event::Resize(Size {
            width: 130,
            height: 300,
        }));
        let narrow = host.frame().unwrap();
        assert!(narrow.scene.ops.iter().any(|op| matches!(
            op,
            misa_pixel_ui::Op::ClipRect {
                x: 36.0,
                y: 125.0,
                width: 58.0,
                ..
            }
        )));
        host.input(Event::Pointer {
            x: 94.0,
            y: 135.0,
            dragging: false,
        });
        assert!(host.dashboard.checked()); // half-open hit bound
        host.input(Event::Pointer {
            x: 40.0,
            y: 135.0,
            dragging: false,
        });
        assert!(!host.dashboard.checked());
        assert_ne!(narrow.pixels, host.frame().unwrap().pixels);
    }

    #[test]
    fn native_gpu_input_and_resize_without_display() {
        let _gpu = GPU.lock().unwrap();
        let mut host = Headless::new(Size {
            width: 500,
            height: 320,
        })
        .expect("Vulkan ICD and GPU readback required; configure VK_ICD_FILENAMES");
        let initial = host.frame().unwrap();
        assert_eq!(pixel(&initial, 0, 0), [20, 22, 26, 255]);
        assert_eq!(pixel(&initial, 50, 160), [65, 74, 86, 255]);
        host.input(Event::Pointer {
            x: 40.0,
            y: 155.0,
            dragging: false,
        });
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
