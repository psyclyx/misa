fn main() -> Result<(), String> {
    match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => misa_pixel_testbed::run(),
        [flag] if flag == "--headless" => misa_pixel_testbed::run(),
        [flag] if flag == "--nested-flow" => misa_pixel_testbed::flow_demo::run(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "Usage: misa-pixel-testbed [--headless | --nested-flow | --help]\nNative offscreen Vulkan GPU readback (no display; Vulkan ICD required)."
            );
            Ok(())
        }
        _ => Err("Unknown arguments; use --help".into()),
    }
}
