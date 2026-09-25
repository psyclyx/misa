//! Standalone offline pixel fixture; builds without the production network stack.
fn main() -> Result<(), String> {
    match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => misa_skia_testbed::testbed::headless::run(),
        [flag] if flag == "--headless" => misa_skia_testbed::testbed::headless::run(),
        [flag] if flag == "--window" => run_window(),
        [flag] if flag == "--bench" => misa_skia_testbed::benchmark::run(),
        [flag] if flag == "--bench-ab" => misa_skia_testbed::benchmark::run_ab(),
        [flag] if flag == "--bench-gpu" => misa_skia_testbed::benchmark::run_gpu(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "Usage: misa-skia-testbed [--headless | --window | --bench | --bench-ab | --bench-gpu | --help]\n\nNo arguments / --headless: offline Vulkan/Ganesh fixture, synthetic input/clock and GPU readback (no display; Vulkan required).\n--window: interactive offline fixture window (requires --features native).\n--bench: offline 10/1000-owner App::frame and Skia raster baseline at 800x600 (no window or network).\n--bench-ab: interleaved cached/uncached warm raster comparison on both fixtures (12 samples per mode).\n--bench-gpu: offscreen Vulkan/Ganesh App::frame_at and GPU render+sync+readback baseline (no native swapchain/full window latency)."
            );
            Ok(())
        }
        _ => Err("Unknown arguments; use --help".into()),
    }
}

#[cfg(feature = "native")]
fn run_window() -> Result<(), String> {
    misa_skia_testbed::testbed::window::run()
}

#[cfg(not(feature = "native"))]
fn run_window() -> Result<(), String> {
    Err("--window requires the native feature; run with `cargo run -p misa-skia-testbed --features native -- --window`".into())
}
