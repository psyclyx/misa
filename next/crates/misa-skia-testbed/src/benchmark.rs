//! Offline baseline for the production semantic App -> Skia raster path.
//! No window, daemon, network, PNG encoding or presenter is involved.
use crate::{
    Scene,
    app::{App, DocumentUpdate},
};
use misa_proto::{
    sync::ViewOp,
    view::{BlobRef, Kind, Node, Span},
};
use misa_skia_paint as paint;
use misa_skia_vulkan::Renderer;
use misa_style::Color;
use std::{
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;
const BACKGROUND: Color = Color::Rgb(20, 22, 26);
const ITERATIONS: usize = 12;
const GPU_CLOCK: Duration = Duration::ZERO;
const GPU_IMAGE_HASH: &str = "b000000000000000000000000000000000000000000000000000000000000001";

/// Stable, id-addressed owners. The last owner is in the followed viewport
/// even for the 1000-owner case.
fn fixture(owners: usize) -> Node {
    Node::section("session")
        .id("session")
        .children((0..owners).map(owner))
}

fn owner(index: usize) -> Node {
    Node::section("message.assistant")
        .id(format!("owner.{index}"))
        .child(
            Node::text(
                "message.assistant",
                [Span::plain(format!(
                    "Owner {index:04}: deterministic offline rendering fixture"
                ))],
            )
            .id(format!("owner.{index}.text")),
        )
}

fn changed_owner(index: usize) -> Node {
    Node::section("message.assistant")
        .id(format!("owner.{index}"))
        .child(
            Node::text(
                "message.assistant",
                [Span::plain(format!(
                    "Owner {index:04}: CHANGED offline rendering fixture"
                ))],
            )
            .id(format!("owner.{index}.text")),
        )
}

fn new_app(view: Node) -> Result<App, String> {
    let metrics =
        paint::text_metrics().map_err(|error| format!("Cannot load Skia text metrics: {error}"))?;
    Ok(App::new(view, metrics))
}

fn frame(app: &mut App) -> (Scene, Duration) {
    let start = Instant::now();
    let scene = app.frame_at(WIDTH, HEIGHT, GPU_CLOCK);
    (black_box(scene), start.elapsed())
}

fn raster(scene: &Scene) -> Result<(image::RgbaImage, Duration), String> {
    let start = Instant::now();
    let pixels = paint::raster(black_box(scene), BACKGROUND)?;
    let elapsed = start.elapsed();
    Ok((black_box(pixels), elapsed))
}

fn raster_profiled(
    scene: &Scene,
    cached: bool,
) -> Result<(image::RgbaImage, Duration, paint::RasterPhases), String> {
    let start = Instant::now();
    let (pixels, phases) = if cached {
        paint::raster_profiled(black_box(scene), BACKGROUND)?
    } else {
        paint::raster_uncached_profiled(black_box(scene), BACKGROUND)?
    };
    let elapsed = start.elapsed();
    Ok((black_box(pixels), elapsed, phases))
}

fn replace_last(app: &mut App, owners: usize) -> Result<(), String> {
    let op = [ViewOp::Replace {
        id: format!("owner.{}", owners - 1),
        node: changed_owner(owners - 1),
    }];
    app.observed(&DocumentUpdate::Changed {
        tree: &op,
        live: &[],
        reset_live: false,
    })
}

// Correctness is outside the timed region. Check six consecutive unchanged
// frames, not just scenes or hashes; also ensure the edit is actually visible.
fn verify(owners: usize) -> Result<(image::RgbaImage, image::RgbaImage), String> {
    let mut app = new_app(fixture(owners))?;
    let (first, _) = frame(&mut app);
    // Pre-cache per-frame resolution is the pixel reference, not another
    // invocation of the optimized path.
    let (expected, _, _) = raster_profiled(&first, false)?;
    if raster(&first)?.0 != expected || raster_profiled(&first, true)?.0 != expected {
        return Err(format!(
            "{owners} owners: first frame differs from uncached reference"
        ));
    }
    if expected.width() != WIDTH || expected.height() != HEIGHT {
        return Err(format!("{owners} owners: incorrect raster dimensions"));
    }
    for index in 1..6 {
        let (scene, _) = frame(&mut app);
        let (reference, _, _) = raster_profiled(&scene, false)?;
        if reference != expected
            || raster(&scene)?.0 != expected
            || raster_profiled(&scene, true)?.0 != expected
        {
            return Err(format!(
                "{owners} owners: unchanged frame {index} differs from uncached reference"
            ));
        }
    }
    replace_last(&mut app, owners)?;
    let (scene, _) = frame(&mut app);
    let (changed, _, _) = raster_profiled(&scene, false)?;
    if raster(&scene)?.0 != changed || raster_profiled(&scene, true)?.0 != changed {
        return Err(format!(
            "{owners} owners: changed frame differs from uncached reference"
        ));
    }
    if changed == expected {
        return Err(format!(
            "{owners} owners: changed owner did not change visible pixels"
        ));
    }
    let (scene, _) = frame(&mut app);
    let (again, _, _) = raster_profiled(&scene, false)?;
    if again != changed
        || raster(&scene)?.0 != changed
        || raster_profiled(&scene, true)?.0 != changed
    {
        return Err(format!("{owners} owners: edited frame is not stable"));
    }
    Ok((expected, changed))
}

#[derive(Default)]
struct RasterSamples {
    total: Vec<Duration>,
    surface_clear: Vec<Duration>,
    font_resolver: Vec<Duration>,
    draw_ops: Vec<Duration>,
    pixel_read_convert: Vec<Duration>,
}

impl RasterSamples {
    fn push(&mut self, elapsed: Duration, phases: paint::RasterPhases) {
        self.total.push(elapsed);
        self.surface_clear.push(phases.surface_clear);
        self.font_resolver.push(phases.font_resolver);
        self.draw_ops.push(phases.draw_ops);
        self.pixel_read_convert.push(phases.pixel_read_convert);
    }

    fn report(self) {
        stats("raster total", self.total);
        stats("  surface/clear", self.surface_clear);
        stats("  font resolver", self.font_resolver);
        stats("  draw_ops", self.draw_ops);
        stats("  pixel read/convert", self.pixel_read_convert);
    }
}

#[derive(Default)]
struct Samples {
    cold_frame: Vec<Duration>,
    cold_raster: RasterSamples,
    warm_frame: Vec<Duration>,
    warm_raster: RasterSamples,
    changed_frame: Vec<Duration>,
    changed_raster: RasterSamples,
}

fn measure(owners: usize, iterations: usize) -> Result<Samples, String> {
    let (expected, changed) = verify(owners)?;
    let mut samples = Samples::default();
    for sample in 0..iterations {
        // Build the same tree/App outside the timed region for each cold-cache
        // sample. Cold means first App::frame_at after App::new, not cold OS/font
        // caches. Raster always creates a fresh surface via paint::raster_profiled.
        let mut app = new_app(fixture(owners))?;
        let (scene, elapsed) = frame(&mut app);
        samples.cold_frame.push(elapsed);
        let (pixels, elapsed, phases) = raster_profiled(&scene, true)?;
        if pixels != expected {
            return Err(format!(
                "{owners} owners: cold profiled sample {sample} differs from uncached reference pixels"
            ));
        }
        samples.cold_raster.push(elapsed, phases);

        let (scene, elapsed) = frame(&mut app);
        samples.warm_frame.push(elapsed);
        let (pixels, elapsed, phases) = raster_profiled(&scene, true)?;
        if pixels != expected {
            return Err(format!(
                "{owners} owners: warm profiled sample {sample} differs from uncached reference pixels"
            ));
        }
        samples.warm_raster.push(elapsed, phases);

        replace_last(&mut app, owners)?;
        let (scene, elapsed) = frame(&mut app);
        samples.changed_frame.push(elapsed);
        let (pixels, elapsed, phases) = raster_profiled(&scene, true)?;
        if pixels != changed {
            return Err(format!(
                "{owners} owners: changed profiled sample {sample} differs from uncached reference pixels"
            ));
        }
        samples.changed_raster.push(elapsed, phases);
    }
    Ok(samples)
}

// The GPU sample includes a small decoded image on the last (visible) owner.
// Keep the CPU fixture unchanged so historical --bench/--bench-ab remain comparable.
fn gpu_image() -> Node {
    Node::new(
        "image",
        Kind::Image {
            blob: BlobRef {
                hash: GPU_IMAGE_HASH.into(),
                len: 0,
                media: Some("image/png".into()),
            },
            alt: "fixture image".into(),
            width: 16,
            height: 16,
        },
    )
    .id("bench-image")
}

fn gpu_owner(index: usize, changed: bool) -> Node {
    (if changed {
        changed_owner(index)
    } else {
        owner(index)
    })
    .child(gpu_image())
}

fn gpu_fixture(owners: usize) -> Node {
    Node::section("session")
        .id("session")
        .children((0..owners).map(|i| {
            if i == owners - 1 {
                gpu_owner(i, false)
            } else {
                owner(i)
            }
        }))
}

fn gpu_app(owners: usize) -> Result<App, String> {
    let mut app = new_app(gpu_fixture(owners))?;
    app.image(
        GPU_IMAGE_HASH.into(),
        Arc::new(image::RgbaImage::from_pixel(
            16,
            16,
            image::Rgba([240, 100, 40, 255]),
        )),
    );
    Ok(app)
}

fn gpu_replace_last(app: &mut App, owners: usize) -> Result<(), String> {
    let op = [ViewOp::Replace {
        id: format!("owner.{}", owners - 1),
        node: gpu_owner(owners - 1, true),
    }];
    app.observed(&DocumentUpdate::Changed {
        tree: &op,
        live: &[],
        reset_live: false,
    })
}

// Separate clocks around App layout and production offscreen Ganesh render.
// Renderer::render includes surface allocation, draw, flush_submit_and_sync_cpu,
// and full RGBA readback; it does NOT time native swapchain presentation.
fn gpu_frame(
    app: &mut App,
    renderer: &mut Renderer,
) -> Result<(image::RgbaImage, Duration, Duration), String> {
    let start = Instant::now();
    let scene = black_box(app.frame_at(WIDTH, HEIGHT, GPU_CLOCK));
    let frame_time = start.elapsed();
    let start = Instant::now();
    let pixels = renderer.render(black_box(&scene), BACKGROUND)?;
    let render_time = start.elapsed();
    Ok((black_box(pixels), frame_time, render_time))
}

fn has_image(ops: &[crate::Op]) -> bool {
    ops.iter().any(|op| {
        matches!(op, crate::Op::Image { .. })
            || matches!(op, crate::Op::Group { ops, .. } | crate::Op::ClipRect { ops, .. } if has_image(ops))
    })
}

// Same device, same clock, six consecutive GPU readbacks in each stable state.
// No CPU raster comparison: GPU and CPU pixels need not be byte-identical.
fn gpu_verify(
    owners: usize,
    renderer: &mut Renderer,
) -> Result<(image::RgbaImage, image::RgbaImage), String> {
    let mut app = gpu_app(owners)?;
    // Guard the image-bearing sample against silently becoming an alt-text-only scene.
    if !has_image(&app.frame_at(WIDTH, HEIGHT, GPU_CLOCK).ops) {
        return Err(format!(
            "{owners} owners: decoded image missing from GPU scene"
        ));
    }
    // Use a new App for the cold correctness frame; the probe above is untimed.
    let mut app = gpu_app(owners)?;
    let mut expected = None;
    for index in 0..6 {
        let (pixels, _, _) = gpu_frame(&mut app, renderer)?;
        if pixels.dimensions() != (WIDTH, HEIGHT) {
            return Err(format!(
                "{owners} owners: incorrect GPU readback dimensions"
            ));
        }
        if expected.as_ref().is_some_and(|first| first != &pixels) {
            return Err(format!(
                "{owners} owners: unchanged GPU frame {index} differs"
            ));
        }
        expected = Some(pixels);
    }
    let expected = expected.expect("six GPU frames");
    gpu_replace_last(&mut app, owners)?;
    let mut changed = None;
    for index in 0..6 {
        let (pixels, _, _) = gpu_frame(&mut app, renderer)?;
        if pixels == expected {
            return Err(format!(
                "{owners} owners: edited owner did not change GPU pixels"
            ));
        }
        if changed.as_ref().is_some_and(|first| first != &pixels) {
            return Err(format!("{owners} owners: edited GPU frame {index} differs"));
        }
        changed = Some(pixels);
    }
    Ok((expected, changed.expect("six edited GPU frames")))
}

#[derive(Default)]
struct GpuPhase {
    frame: Vec<Duration>,
    render_sync_readback: Vec<Duration>,
}

impl GpuPhase {
    fn push(&mut self, frame: Duration, render: Duration) {
        self.frame.push(frame);
        self.render_sync_readback.push(render);
    }

    fn report(self, label: &str) {
        println!("  {label}:");
        stats("App::frame_at/layout", self.frame);
        stats("GPU render+sync+readback", self.render_sync_readback);
    }
}

/// Offscreen Ganesh baseline on one production Vulkan Renderer per run.
/// App construction, edits, pixel comparisons and device setup are untimed.
pub fn run_gpu() -> Result<(), String> {
    let mut renderer = Renderer::new()?;
    println!(
        "Skia Vulkan/Ganesh offscreen baseline | {} | device: {} | {WIDTH}x{HEIGHT} | n={ITERATIONS}/phase",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        renderer.device_name
    );
    println!(
        "Cold = first frame of fresh App (not cold OS/driver/device); warm = unchanged; changed = replace last visible owner. Fixed App::frame_at clock, decoded image fixture. Oracle = six same-device GPU readbacks per unchanged state and a visible edit, NOT CPU pixel parity. GPU render total includes allocation/draw/sync/RGBA readback. No native swapchain or full window latency measured; no performance comparisons implied."
    );
    for owners in [10, 1000] {
        let (expected, changed) = gpu_verify(owners, &mut renderer)?;
        let mut cold = GpuPhase::default();
        let mut warm = GpuPhase::default();
        let mut replaced = GpuPhase::default();
        for sample in 0..ITERATIONS {
            let mut app = gpu_app(owners)?;
            let (pixels, frame, render) = gpu_frame(&mut app, &mut renderer)?;
            if pixels != expected {
                return Err(format!(
                    "{owners} owners: cold GPU sample {sample} pixel mismatch"
                ));
            }
            cold.push(frame, render);
            let (pixels, frame, render) = gpu_frame(&mut app, &mut renderer)?;
            if pixels != expected {
                return Err(format!(
                    "{owners} owners: warm GPU sample {sample} pixel mismatch"
                ));
            }
            warm.push(frame, render);
            gpu_replace_last(&mut app, owners)?;
            let (pixels, frame, render) = gpu_frame(&mut app, &mut renderer)?;
            if pixels != changed {
                return Err(format!(
                    "{owners} owners: changed GPU sample {sample} pixel mismatch"
                ));
            }
            replaced.push(frame, render);
        }
        println!(
            "{owners} owners (six same-device identical readbacks per stable state; edit visible):"
        );
        cold.report("cold");
        warm.report("warm");
        replaced.report("replace changed");
    }
    Ok(())
}

fn stats(label: &str, mut times: Vec<Duration>) {
    times.sort_unstable();
    let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
    let median = (ms(times[(times.len() - 1) / 2]) + ms(times[times.len() / 2])) / 2.0;
    println!(
        "  {label:<22} median {:>9.3} ms  best {:>9.3} ms  n={}",
        median,
        ms(times[0]),
        times.len()
    );
}

/// Same-binary A/B on the same warm scene: alternating ABBA/BAAB blocks = 12 samples
/// per mode per fixture. Pixel checks, scene construction and warmup are not
/// part of the timed raster; phase clocks are identical in both modes.
pub fn run_ab() -> Result<(), String> {
    println!(
        "Skia cached vs uncached font A/B | {} | {WIDTH}x{HEIGHT} | warm raster | 6 alternating ABBA/BAAB blocks, n=12/mode",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    for owners in [10, 1000] {
        let (expected, _) = verify(owners)?;
        let mut app = new_app(fixture(owners))?;
        let _ = frame(&mut app);
        let (scene, _) = frame(&mut app);
        let mut cached = RasterSamples::default();
        let mut uncached = RasterSamples::default();
        // Warm both Skia paths and the OS font cache before timing.
        for mode in [true, false] {
            if raster_profiled(&scene, mode)?.0 != expected {
                return Err(format!("{owners} owners: A/B warmup pixel mismatch"));
            }
        }
        for block in 0..6 {
            let order = if block % 2 == 0 {
                [true, false, false, true]
            } else {
                [false, true, true, false]
            };
            for (sample, mode) in order.into_iter().enumerate() {
                let (pixels, elapsed, phases) = raster_profiled(&scene, mode)?;
                if pixels != expected {
                    return Err(format!(
                        "{owners} owners: A/B block {block} sample {sample} pixel mismatch"
                    ));
                }
                if mode {
                    cached.push(elapsed, phases);
                } else {
                    uncached.push(elapsed, phases);
                }
            }
        }
        println!("{owners} owners (identical warm scene; uncached reference verified):");
        println!("  cache on:");
        cached.report();
        println!("  cache off:");
        uncached.report();
    }
    Ok(())
}

/// An explicit, measured baseline; never invoked by the interactive testbed.
pub fn run() -> Result<(), String> {
    println!(
        "Skia offline App::frame -> profiled paint::raster baseline | {} | {WIDTH}x{HEIGHT} | {ITERATIONS} iterations/phase",
        if cfg!(debug_assertions) {
            "debug (debug_assertions)"
        } else {
            "release (no debug_assertions)"
        }
    );
    println!(
        "Cold = first frame of fresh App; warm = unchanged frame; changed = replace last visible owner. App construction, update, correctness checks and PNG/presentation excluded. Raster total uses profiled path; phases are independent medians and need not sum to total."
    );
    for owners in [10, 1000] {
        let samples = measure(owners, ITERATIONS)?;
        println!("{owners} owners (six unchanged frames pixel-identical; changed owner visible):");
        stats("cold frame", samples.cold_frame);
        println!("  cold raster:");
        samples.cold_raster.report();
        stats("warm frame", samples.warm_frame);
        println!("  warm raster:");
        samples.warm_raster.report();
        stats("changed frame", samples.changed_frame);
        println!("  changed raster:");
        samples.changed_raster.report();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_fixture_and_offline_pipeline_smoke() {
        assert_eq!(fixture(10), fixture(10));
        let scene = new_app(fixture(10))
            .expect("Skia text metrics for benchmark test")
            .frame_at(WIDTH, HEIGHT, GPU_CLOCK);
        assert_eq!(
            paint::raster(&scene, BACKGROUND).unwrap(),
            paint::raster_profiled(&scene, BACKGROUND).unwrap().0,
            "profiled and production paint must return identical pixels for the same scene"
        );
        // Exercises uncached vs production and profiled pixels on six
        // unchanged frames plus the visible edit, before any timing claim.
        let result = measure(10, 1).expect("raster and six-frame oracle");
        assert_eq!(result.cold_frame.len(), 1);
        assert_eq!(result.warm_raster.total.len(), 1);
        assert_eq!(result.warm_raster.draw_ops.len(), 1);
        assert_eq!(result.changed_frame.len(), 1);
    }

    #[test]
    fn concurrent_rasters_share_the_cached_typeface() {
        let scene = new_app(fixture(10))
            .expect("Skia text metrics for benchmark test")
            .frame_at(WIDTH, HEIGHT, GPU_CLOCK);
        let reference = paint::raster_uncached_profiled(&scene, BACKGROUND)
            .unwrap()
            .0;
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| paint::raster(&scene, BACKGROUND)))
                .collect();
            for handle in handles {
                assert_eq!(handle.join().unwrap().unwrap(), reference);
            }
        });
    }
}
