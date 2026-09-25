//! Direct Ganesh rendering into acquired Vulkan swapchain images.
use super::{Renderer, error};
use ash::{vk, vk::Handle};
use misa_render::Color;
use misa_skia_paint::draw_scene;
use misa_skia_ui::Scene;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use skia_safe::{ColorType, gpu};
use std::ffi::CString;

struct Chain {
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    extent: vk::Extent2D,
    requested: (u32, u32),
    format: vk::Format,
}

// Dirty retains an old chain solely for old_swapchain and cleanup, never for rendering.
// Reconfigure owns no WSI resources. Poisoned retains resources for cleanup at drop,
// but forbids another acquire after a failure with an acquired image.
enum ChainState {
    Reconfigure,
    Ready(Chain),
    Dirty(Chain),
    Poisoned(Chain),
}

impl ChainState {
    fn mark_dirty(&mut self) {
        *self = match std::mem::replace(self, Self::Reconfigure) {
            Self::Ready(chain) | Self::Dirty(chain) => Self::Dirty(chain),
            other => other,
        };
    }

    fn retire(&mut self) -> Option<Chain> {
        match std::mem::replace(self, Self::Reconfigure) {
            Self::Ready(chain) | Self::Dirty(chain) => Some(chain),
            Self::Reconfigure => None,
            Self::Poisoned(chain) => {
                *self = Self::Poisoned(chain);
                None
            }
        }
    }

    fn poison(&mut self) {
        *self = match std::mem::replace(self, Self::Reconfigure) {
            Self::Ready(chain) | Self::Dirty(chain) | Self::Poisoned(chain) => {
                Self::Poisoned(chain)
            }
            Self::Reconfigure => Self::Reconfigure,
        };
    }

    fn after_acquired(
        &mut self,
        result: Result<bool, PresentFailure>,
        suboptimal: bool,
    ) -> Result<PresentOutcome, String> {
        let outcome = match result {
            Ok(outdated) => presented_outcome(outdated, suboptimal),
            Err(PresentFailure::OutOfDate) => PresentOutcome::Retry,
            Err(PresentFailure::Fatal(message)) => {
                self.poison();
                return Err(message);
            }
        };
        if outcome.needs_redraw() {
            self.mark_dirty();
        }
        Ok(outcome)
    }
}

// Only queue_present's OUT_OF_DATE is recoverable after acquisition. Everything
// before a completed submit, and other present errors, leaves ownership uncertain.
#[derive(Debug, PartialEq, Eq)]
enum PresentFailure {
    OutOfDate,
    Fatal(String),
}

impl From<String> for PresentFailure {
    fn from(message: String) -> Self {
        Self::Fatal(message)
    }
}

impl From<&'static str> for PresentFailure {
    fn from(message: &'static str) -> Self {
        Self::Fatal(message.into())
    }
}

fn classify_present(result: Result<bool, vk::Result>) -> Result<bool, PresentFailure> {
    match result {
        Ok(outdated) => Ok(outdated),
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Err(PresentFailure::OutOfDate),
        Err(e) => Err(PresentFailure::Fatal(error("queue present", e))),
    }
}

/// Whether this paint reached the current swapchain. Retry is a hint for one
/// additional redraw, not permission to spin on a persistently outdated surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented,
    Retry,
    SkippedZero,
}

impl PresentOutcome {
    pub fn needs_redraw(self) -> bool {
        matches!(self, Self::Retry)
    }
}

pub struct WindowRenderer {
    renderer: Renderer,
    surfaces: ash::khr::surface::Instance,
    swaps: ash::khr::swapchain::Device,
    state: ChainState,
    acquire_fence: vk::Fence,
}

impl WindowRenderer {
    /// The caller keeps both handles' window and display alive until this renderer is dropped.
    pub fn new(display: RawDisplayHandle, window: RawWindowHandle) -> Result<Self, String> {
        let entry = unsafe { ash::Entry::load() }.map_err(|e| error("Vulkan loader", e))?;
        let extensions = ash_window::enumerate_required_extensions(display)
            .map_err(|e| error("surface extensions", e))?;
        let name = CString::new("misa-skia-vulkan").unwrap();
        let app = vk::ApplicationInfo::default()
            .application_name(&name)
            .api_version(vk::API_VERSION_1_1);
        let info = vk::InstanceCreateInfo::default()
            .application_info(&app)
            .enabled_extension_names(extensions);
        let instance = unsafe { entry.create_instance(&info, None) }
            .map_err(|e| error("Vulkan instance", e))?;
        let surface =
            match unsafe { ash_window::create_surface(&entry, &instance, display, window, None) } {
                Ok(surface) => surface,
                Err(e) => {
                    unsafe { instance.destroy_instance(None) };
                    return Err(error("window surface", e));
                }
            };
        let renderer = Renderer::with_instance(entry, instance, Some(surface))?;
        let surfaces = ash::khr::surface::Instance::new(&renderer._entry, &renderer.instance);
        let swaps = ash::khr::swapchain::Device::new(&renderer.instance, &renderer.device);
        let fence = unsafe {
            renderer
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        };
        let acquire_fence = fence.map_err(|e| error("acquire fence", e))?;
        Ok(Self {
            renderer,
            surfaces,
            swaps,
            state: ChainState::Reconfigure,
            acquire_fence,
        })
    }

    fn destroy_chain(&self, chain: Chain) {
        unsafe { self.swaps.destroy_swapchain(chain.swapchain, None) }
    }

    fn configure(&mut self, width: u32, height: u32) -> Result<(), String> {
        if matches!(self.state, ChainState::Poisoned(_)) {
            return Err("swapchain is unusable after an acquired-frame failure".into());
        }
        // A failed configure must be retried even when the requested size is unchanged.
        self.state.mark_dirty();
        let surface = self.renderer.surface.expect("window surface");
        let physical = self.renderer.physical;
        let caps = unsafe {
            self.surfaces
                .get_physical_device_surface_capabilities(physical, surface)
        }
        .map_err(|e| error("surface capabilities", e))?;
        let formats = unsafe {
            self.surfaces
                .get_physical_device_surface_formats(physical, surface)
        }
        .map_err(|e| error("surface formats", e))?;
        let chosen = formats
            .iter()
            .find(|f| {
                let format = if f.format == vk::Format::UNDEFINED {
                    vk::Format::B8G8R8A8_UNORM
                } else {
                    f.format
                };
                matches!(
                    format,
                    vk::Format::B8G8R8A8_UNORM | vk::Format::R8G8B8A8_UNORM
                ) && unsafe {
                    self.renderer
                        .instance
                        .get_physical_device_format_properties(physical, format)
                }
                .optimal_tiling_features
                .contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT)
            })
            .ok_or("no renderable RGBA/BGRA UNORM swapchain format supported")?;
        let format = if chosen.format == vk::Format::UNDEFINED {
            vk::Format::B8G8R8A8_UNORM
        } else {
            chosen.format
        };
        let extent = choose_extent(&caps, width, height);
        if extent.width == 0
            || extent.height == 0
            || extent.width > i32::MAX as u32
            || extent.height > i32::MAX as u32
        {
            return Err("invalid swapchain extent".into());
        }
        let modes = unsafe {
            self.surfaces
                .get_physical_device_surface_present_modes(physical, surface)
        }
        .map_err(|e| error("present modes", e))?;
        if !modes.contains(&vk::PresentModeKHR::FIFO) {
            return Err("FIFO presentation unavailable".into());
        }
        if !caps
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        {
            return Err("surface does not support color attachment rendering".into());
        }
        let transform = if caps
            .supported_transforms
            .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
        {
            vk::SurfaceTransformFlagsKHR::IDENTITY
        } else {
            caps.current_transform
        };
        let alpha = [
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::INHERIT,
        ]
        .into_iter()
        .find(|a| caps.supported_composite_alpha.contains(*a))
        .ok_or("no composite alpha mode")?;
        let old = match &self.state {
            ChainState::Dirty(chain) => chain.swapchain,
            ChainState::Reconfigure => vk::SwapchainKHR::null(),
            _ => unreachable!(),
        };
        let create = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count(&caps))
            .image_format(format)
            .image_color_space(chosen.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(transform)
            .composite_alpha(alpha)
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true)
            .old_swapchain(old);
        // A synchronous renderer: no image or Skia surface can still refer to the old chain.
        unsafe { self.renderer.device.device_wait_idle() }
            .map_err(|e| error("swapchain idle", e))?;
        self.renderer
            .context
            .as_mut()
            .expect("live context")
            .free_gpu_resources();
        let swapchain = unsafe { self.swaps.create_swapchain(&create, None) }
            .map_err(|e| error("create swapchain", e))?;
        // Success retires old_swapchain even if discovering images fails.
        // Remove it from the usable state before any fallible post-create operation.
        if let Some(retired) = self.state.retire() {
            self.destroy_chain(retired);
        }
        let images = match unsafe { self.swaps.get_swapchain_images(swapchain) } {
            Ok(images) => images,
            Err(e) => {
                unsafe { self.swaps.destroy_swapchain(swapchain, None) };
                return Err(error("swapchain images", e));
            }
        };
        self.state = ChainState::Ready(Chain {
            swapchain,
            images,
            extent,
            requested: (width, height),
            format,
        });
        Ok(())
    }

    /// Returns SkippedZero when minimized, or Retry when recreation left the new
    /// swapchain without a painted image. Snapshot readback is independent: use
    /// `renderer().render()` explicitly only when the caller requests an export.
    pub fn present(
        &mut self,
        scene: &Scene,
        background: Color,
        width: u32,
        height: u32,
    ) -> Result<PresentOutcome, String> {
        if matches!(self.state, ChainState::Poisoned(_)) {
            return Err("swapchain is unusable after an acquired-frame failure".into());
        }
        if width == 0 || height == 0 {
            return Ok(PresentOutcome::SkippedZero);
        }
        if !matches!(&self.state, ChainState::Ready(chain) if chain.requested == (width, height)) {
            self.configure(width, height)?;
        }
        let swapchain = match &self.state {
            ChainState::Ready(chain) => chain.swapchain,
            _ => unreachable!(),
        };
        let (index, suboptimal) = match unsafe {
            self.swaps.acquire_next_image(
                swapchain,
                u64::MAX,
                vk::Semaphore::null(),
                self.acquire_fence,
            )
        } {
            Ok(pair) => pair,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.configure(width, height)?;
                return Ok(PresentOutcome::Retry);
            }
            Err(e) => return Err(error("acquire image", e)),
        };
        // Only queue_present OUT_OF_DATE is recoverable after acquisition.
        let result = self.present_acquired(scene, background, index as usize);
        let outcome = self.state.after_acquired(result, suboptimal)?;
        if outcome.needs_redraw() {
            self.configure(width, height)?;
        }
        Ok(outcome)
    }

    fn present_acquired(
        &mut self,
        scene: &Scene,
        background: Color,
        index: usize,
    ) -> Result<bool, PresentFailure> {
        unsafe {
            self.renderer
                .device
                .wait_for_fences(&[self.acquire_fence], true, u64::MAX)
                .map_err(|e| error("acquire wait", e))?;
            self.renderer
                .device
                .reset_fences(&[self.acquire_fence])
                .map_err(|e| error("acquire reset", e))?;
        }
        let chain = match &self.state {
            ChainState::Ready(chain) => chain,
            _ => unreachable!(),
        };
        let image = *chain
            .images
            .get(index)
            .ok_or("invalid acquired image index")?;
        let extent = chain.extent;
        let format = chain.format;
        let swapchain = chain.swapchain;
        // The acquired image's previous contents are discarded on every frame.
        let layout = gpu::vk::ImageLayout::UNDEFINED;
        let color_type = if format == vk::Format::B8G8R8A8_UNORM {
            ColorType::BGRA8888
        } else {
            ColorType::RGBA8888
        };
        // Swapchain images are owned by the presentation engine; Skia must never free them.
        let info = unsafe {
            gpu::vk::ImageInfo::new(
                image.as_raw() as gpu::vk::Image,
                gpu::vk::Alloc::default(),
                gpu::vk::ImageTiling::OPTIMAL,
                layout,
                if format == vk::Format::B8G8R8A8_UNORM {
                    gpu::vk::Format::B8G8R8A8_UNORM
                } else {
                    gpu::vk::Format::R8G8B8A8_UNORM
                },
                1,
                None,
                None,
                None,
                None,
            )
        };
        let target = gpu::backend_render_targets::make_vk(
            (extent.width as i32, extent.height as i32),
            &info,
        );
        let context = self.renderer.context.as_mut().expect("live context");
        let mut surface = gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            gpu::SurfaceOrigin::TopLeft,
            color_type,
            None,
            None,
        )
        .ok_or("Ganesh cannot wrap the swapchain image")?;
        draw_scene(surface.canvas(), scene, background)?;
        let present_state = gpu::vk::mutable_texture_states::new_vulkan(
            gpu::vk::ImageLayout::PRESENT_SRC_KHR,
            self.renderer.queue_family,
        );
        context.flush_surface_with_texture_state(
            &mut surface,
            &gpu::FlushInfo::default(),
            Some(&present_state),
        );
        if !context.submit(gpu::SyncCpu::Yes) {
            return Err("Ganesh Vulkan submit failed".into());
        }
        drop(surface);
        // SyncCpu::Yes waits for Ganesh's work; no separate WSI signal is needed.
        let swaps = [swapchain];
        let indices = [index as u32];
        let present = vk::PresentInfoKHR::default()
            .swapchains(&swaps)
            .image_indices(&indices);
        classify_present(unsafe {
            self.swaps
                .queue_present(self.renderer.graphics_queue, &present)
        })
    }

    pub fn renderer(&mut self) -> &mut Renderer {
        &mut self.renderer
    }
}

fn presented_outcome(outdated: bool, suboptimal: bool) -> PresentOutcome {
    if outdated || suboptimal {
        PresentOutcome::Retry
    } else {
        PresentOutcome::Presented
    }
}

fn choose_extent(caps: &vk::SurfaceCapabilitiesKHR, width: u32, height: u32) -> vk::Extent2D {
    if caps.current_extent.width != u32::MAX {
        caps.current_extent
    } else {
        vk::Extent2D {
            width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
            height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
        }
    }
}

fn image_count(caps: &vk::SurfaceCapabilitiesKHR) -> u32 {
    caps.min_image_count
        .saturating_add(1)
        .max(2)
        .min(if caps.max_image_count == 0 {
            u32::MAX
        } else {
            caps.max_image_count
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chain(id: u64) -> Chain {
        Chain {
            swapchain: vk::SwapchainKHR::from_raw(id),
            images: vec![vk::Image::from_raw(id + 1)],
            extent: vk::Extent2D {
                width: 1,
                height: 1,
            },
            requested: (1, 1),
            format: vk::Format::B8G8R8A8_UNORM,
        }
    }

    #[test]
    fn failed_configure_keeps_old_chain_dirty_until_successfully_replaced() {
        let mut state = ChainState::Ready(chain(1));
        state.mark_dirty(); // Even if pre-create configure fails, same-size redraw must retry.
        assert!(matches!(&state, ChainState::Dirty(chain) if chain.requested == (1, 1)));
        state.mark_dirty(); // Repeated failures do not make the old chain renderable.
        assert!(matches!(state, ChainState::Dirty(_)));
        let retired = state.retire().unwrap(); // Successful create retires old_swapchain.
        assert_eq!(retired.swapchain, vk::SwapchainKHR::from_raw(1));
        assert!(matches!(state, ChainState::Reconfigure));
        // Failed get_swapchain_images cannot restore the retired chain.
        assert!(state.retire().is_none());
        state = ChainState::Ready(chain(4));
        assert!(matches!(state, ChainState::Ready(_)));
    }

    #[test]
    fn present_out_of_date_is_recoverable_but_unknown_errors_poison() {
        let mut state = ChainState::Ready(chain(7));
        assert_eq!(
            state.after_acquired(
                classify_present(Err(vk::Result::ERROR_OUT_OF_DATE_KHR)),
                false
            ),
            Ok(PresentOutcome::Retry)
        );
        assert!(matches!(&state, ChainState::Dirty(chain)
            if chain.swapchain == vk::SwapchainKHR::from_raw(7)));
        // If immediate configure fails, next redraw still cannot paint the dirty chain.
        state.mark_dirty();
        assert!(matches!(state, ChainState::Dirty(_)));
        for failure in [
            PresentFailure::Fatal("acquire wait".into()),
            PresentFailure::Fatal("wrap".into()),
            PresentFailure::Fatal("draw".into()),
            PresentFailure::Fatal("submit".into()),
            classify_present(Err(vk::Result::ERROR_DEVICE_LOST)).unwrap_err(),
        ] {
            let mut state = ChainState::Ready(chain(8));
            assert!(state.after_acquired(Err(failure), false).is_err());
            assert!(matches!(&state, ChainState::Poisoned(chain)
                if chain.swapchain == vk::SwapchainKHR::from_raw(8) && chain.images.len() == 1));
            assert!(state.retire().is_none());
            assert!(matches!(state, ChainState::Poisoned(_)));
        }
    }

    #[test]
    fn only_recreated_chains_need_another_paint() {
        let mut state = ChainState::Ready(chain(1));
        assert_eq!(
            state.after_acquired(Ok(false), false),
            Ok(PresentOutcome::Presented)
        );
        assert!(matches!(state, ChainState::Ready(_)));
        for (outdated, suboptimal) in [(true, false), (false, true), (true, true)] {
            let mut state = ChainState::Ready(chain(1));
            assert_eq!(
                state.after_acquired(Ok(outdated), suboptimal),
                Ok(PresentOutcome::Retry)
            );
            assert!(matches!(state, ChainState::Dirty(_)));
        }
        assert!(!PresentOutcome::Presented.needs_redraw());
        assert!(PresentOutcome::Retry.needs_redraw()); // also acquire OUT_OF_DATE
        assert!(!PresentOutcome::SkippedZero.needs_redraw());
    }

    #[test]
    fn fixed_and_variable_extent_and_bounded_image_count() {
        let mut caps = vk::SurfaceCapabilitiesKHR::default();
        caps.min_image_extent = vk::Extent2D {
            width: 100,
            height: 80,
        };
        caps.max_image_extent = vk::Extent2D {
            width: 800,
            height: 600,
        };
        caps.current_extent = vk::Extent2D {
            width: u32::MAX,
            height: u32::MAX,
        };
        let selected = choose_extent(&caps, 50, 1000);
        assert_eq!((selected.width, selected.height), (100, 600));
        caps.current_extent = vk::Extent2D {
            width: 700,
            height: 500,
        };
        let selected = choose_extent(&caps, 50, 1000);
        assert_eq!((selected.width, selected.height), (700, 500));
        caps.min_image_count = 2;
        caps.max_image_count = 2;
        assert_eq!(image_count(&caps), 2);
        caps.max_image_count = 0;
        assert_eq!(image_count(&caps), 3);
    }
}

impl Drop for WindowRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.renderer.device.device_wait_idle();
            self.renderer
                .context
                .as_mut()
                .expect("live context")
                .free_gpu_resources();
            let chain = match std::mem::replace(&mut self.state, ChainState::Reconfigure) {
                ChainState::Ready(chain)
                | ChainState::Dirty(chain)
                | ChainState::Poisoned(chain) => Some(chain),
                ChainState::Reconfigure => None,
            };
            if let Some(chain) = chain {
                self.destroy_chain(chain);
            }
            self.renderer.device.destroy_fence(self.acquire_fence, None);
        }
    }
}
