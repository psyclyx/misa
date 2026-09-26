//! Ganesh Vulkan rendering to offscreen images or native swapchains.
//! The Vulkan device outlives Ganesh; Ganesh owns its offscreen render targets.

use ash::{vk, vk::Handle};
#[cfg(feature = "window")]
mod window;
use misa_pixel_ui::Scene;
use misa_skia_paint::draw_scene;
use misa_style::Color;
use skia_safe::{AlphaType, ColorType, ImageInfo, gpu};
use std::{
    ffi::{CStr, CString},
    ptr,
    sync::{Mutex, MutexGuard, OnceLock},
};

// Ash's loaded entry owns libvulkan. Keep it resident even after the last
// renderer goes away: the loader may still have ICD state in other threads.
static ENTRY: OnceLock<Result<ash::Entry, String>> = OnceLock::new();
static INSTANCE_LIFETIME: Mutex<()> = Mutex::new(());

fn loader_entry() -> Result<&'static ash::Entry, String> {
    ENTRY
        .get_or_init(|| unsafe { ash::Entry::load() }.map_err(|e| error("Vulkan loader", e)))
        .as_ref()
        .map_err(Clone::clone)
}

fn instance_lifetime() -> MutexGuard<'static, ()> {
    // A poisoned lock does not make Vulkan teardown optional. No user code runs
    // while holding it; still serialize cleanup after a panic.
    INSTANCE_LIFETIME.lock().unwrap_or_else(|e| e.into_inner())
}
#[cfg(feature = "window")]
pub use window::WindowRenderer;

fn error(phase: &str, err: impl std::fmt::Display) -> String {
    format!("{phase}: {err}")
}

/// One Vulkan device and a genuine Ganesh direct context. Not a raster fallback.
/// Keep on the creating thread: Skia's Vulkan procedure resolver is thread-local.
pub struct Renderer {
    // Drop order is explicit: Skia must release GPU resources before the Vulkan device.
    context: Option<gpu::DirectContext>,
    device: ash::Device,
    instance: ash::Instance,
    _entry: &'static ash::Entry,
    physical: vk::PhysicalDevice,
    graphics_queue: vk::Queue,
    surface: Option<vk::SurfaceKHR>,
    pub device_name: String,
    pub queue_family: u32,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Renderer {
    /// Load the system Vulkan loader and choose a graphics-capable device.
    /// No ICD or no compatible device is an error, never a CPU fallback.
    pub fn new() -> Result<Self, String> {
        let _lifetime = instance_lifetime();
        let entry = loader_entry()?;
        let name = CString::new("misa-skia-vulkan").expect("static name");
        let app = vk::ApplicationInfo::default()
            .application_name(&name)
            .api_version(vk::API_VERSION_1_1);
        let create = vk::InstanceCreateInfo::default().application_info(&app);
        // SAFETY: Vulkan structs and referenced application name live across the call.
        let instance = unsafe { entry.create_instance(&create, None) }
            .map_err(|e| error("Vulkan instance", e))?;
        let result = Self::with_instance(entry, instance, None);
        result
    }

    fn with_instance(
        entry: &'static ash::Entry,
        instance: ash::Instance,
        surface: Option<vk::SurfaceKHR>,
    ) -> Result<Self, String> {
        // SAFETY: valid instance, and no other threads mutate it.
        let physical_devices = match unsafe { instance.enumerate_physical_devices() } {
            Ok(devices) => devices,
            Err(e) => {
                if let Some(surface) = surface {
                    unsafe {
                        ash::khr::surface::Instance::new(&entry, &instance)
                            .destroy_surface(surface, None)
                    };
                }
                unsafe { instance.destroy_instance(None) };
                return Err(error("enumerate devices", e));
            }
        };
        let chosen = unsafe {
            physical_devices.into_iter().find_map(|physical| {
                let properties = instance.get_physical_device_properties(physical);
                let queues = instance.get_physical_device_queue_family_properties(physical);
                let family = queues
                    .iter()
                    .enumerate()
                    .find(|(index, q)| {
                        q.queue_count > 0
                            && q.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                            && surface.is_none_or(|surface| {
                                ash::khr::surface::Instance::new(&entry, &instance)
                                    .get_physical_device_surface_support(
                                        physical,
                                        *index as u32,
                                        surface,
                                    )
                                    .unwrap_or(false)
                            })
                    })?
                    .0 as u32;
                if surface.is_some()
                    && !instance
                        .enumerate_device_extension_properties(physical)
                        .ok()?
                        .iter()
                        .any(|e| {
                            CStr::from_ptr(e.extension_name.as_ptr()) == ash::khr::swapchain::NAME
                        })
                {
                    return None;
                }
                let format = instance
                    .get_physical_device_format_properties(physical, vk::Format::R8G8B8A8_UNORM);
                if !format
                    .optimal_tiling_features
                    .contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT)
                {
                    return None;
                }
                Some((physical, family, properties))
            })
        };
        let result = (|| {
            let (physical, family, properties) =
                chosen.ok_or("no Vulkan graphics device with RGBA render targets")?;
            let priorities = [1.0];
            let queue = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&priorities)];
            let extensions = [ash::khr::swapchain::NAME.as_ptr()];
            let info = vk::DeviceCreateInfo::default().queue_create_infos(&queue);
            let info = if surface.is_some() {
                info.enabled_extension_names(&extensions)
            } else {
                info
            };
            // SAFETY: physical device and family were enumerated from this instance.
            let device = unsafe { instance.create_device(physical, &info, None) }
                .map_err(|e| error("Vulkan device", e))?;
            let result = (|| {
                // SAFETY: queue 0 was requested from a nonempty graphics family.
                let graphics_queue = unsafe { device.get_device_queue(family, 0) };
                let proc = |request: gpu::vk::GetProcOf| -> *const std::ffi::c_void {
                    // Ash and Skia both use Vulkan's opaque handles. `as_raw()` preserves their bits.
                    let function = unsafe {
                        match request {
                            gpu::vk::GetProcOf::Instance(handle, name) => entry
                                .get_instance_proc_addr(
                                    vk::Instance::from_raw(handle as usize as u64),
                                    name,
                                ),
                            gpu::vk::GetProcOf::Device(_handle, name) => {
                                instance.get_device_proc_addr(device.handle(), name)
                            }
                        }
                    };
                    function.map_or(ptr::null(), |f| f as *const std::ffi::c_void)
                };
                // SAFETY: all four Vulkan handles remain alive through the context's lifetime.
                // The resolver is only used synchronously during Ganesh initialization and calls.
                let mut backend = unsafe {
                    gpu::vk::BackendContext::new(
                        instance.handle().as_raw() as usize as gpu::vk::Instance,
                        physical.as_raw() as usize as gpu::vk::PhysicalDevice,
                        device.handle().as_raw() as usize as gpu::vk::Device,
                        (
                            graphics_queue.as_raw() as usize as gpu::vk::Queue,
                            family as usize,
                        ),
                        &proc,
                    )
                };
                // The instance/device were created for 1.1. A newer physical device
                // must not make Ganesh request unenabled Vulkan 1.2/1.3 entry points.
                backend.set_max_api_version(gpu::vk::Version::new(1, 1, 0));
                let context = gpu::direct_contexts::make_vulkan(&backend, None)
                    .ok_or("Ganesh Vulkan context initialization failed")?;
                if context.backend() != gpu::BackendAPI::Vulkan {
                    return Err("Ganesh did not create a Vulkan context".to_string());
                }
                let device_name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
                    .to_string_lossy()
                    .into_owned();
                Ok((context, device_name, graphics_queue))
            })();
            match result {
                Ok((context, device_name, graphics_queue)) => Ok((
                    context,
                    device,
                    device_name,
                    family,
                    graphics_queue,
                    physical,
                )),
                Err(e) => {
                    unsafe { device.destroy_device(None) };
                    Err(e)
                }
            }
        })();
        match result {
            Ok((context, device, device_name, family, graphics_queue, physical)) => Ok(Self {
                context: Some(context),
                device,
                instance,
                _entry: entry,
                physical,
                graphics_queue,
                surface,
                device_name,
                queue_family: family,
                _thread_bound: std::marker::PhantomData,
            }),
            Err(e) => {
                if let Some(surface) = surface {
                    unsafe {
                        ash::khr::surface::Instance::new(&entry, &instance)
                            .destroy_surface(surface, None)
                    };
                }
                unsafe { instance.destroy_instance(None) };
                Err(e)
            }
        }
    }

    /// Draw scene ops with the shared Skia canvas painter, then explicitly read back
    /// straight RGBA for requested snapshots (never for window presentation).
    pub fn render(&mut self, scene: &Scene, background: Color) -> Result<image::RgbaImage, String> {
        let width = scene.width.ceil().max(1.0) as i32;
        let height = scene.height.ceil().max(1.0) as i32;
        let info = ImageInfo::new(
            (width, height),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let context = self.context.as_mut().expect("live context");
        let mut surface = gpu::surfaces::render_target(
            context,
            gpu::Budgeted::Yes,
            &info,
            None,
            Some(gpu::SurfaceOrigin::TopLeft),
            None,
            None,
            None,
        )
        .ok_or("Ganesh could not allocate a GPU render target")?;
        draw_scene(surface.canvas(), scene, background)?;
        context.flush_submit_and_sync_cpu();
        let row_bytes = width as usize * 4;
        let mut bytes = vec![0u8; row_bytes * height as usize];
        let read_info = ImageInfo::new(
            (width, height),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        if !surface.read_pixels(&read_info, &mut bytes, row_bytes, (0, 0)) {
            return Err("Ganesh GPU RGBA readback failed".into());
        }
        image::RgbaImage::from_raw(width as u32, height as u32, bytes)
            .ok_or("invalid RGBA readback".into())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _lifetime = instance_lifetime();
        unsafe {
            let _ = self.device.device_wait_idle();
        }
        self.context.take();
        unsafe {
            self.device.destroy_device(None);
            if let Some(surface) = self.surface {
                ash::khr::surface::Instance::new(&self._entry, &self.instance)
                    .destroy_surface(surface, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}
