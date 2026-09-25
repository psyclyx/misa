//! CLI contract: offscreen is the default, and the display adapter is opt-in.
use std::process::Command;

fn invoke(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_misa-skia-testbed"))
        .args(args)
        .output()
        .expect("testbed binary")
}

#[test]
fn default_and_headless_alias_render_offscreen() {
    for args in [&[][..], &["--headless"][..]] {
        let output = invoke(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Headless Vulkan/Ganesh:"));
        assert!(stdout.contains("Offscreen RGBA readback: 600x400"));
    }
}

#[cfg(not(feature = "native"))]
#[test]
fn window_requires_explicit_native_feature() {
    let output = invoke(&["--window"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--features native"));
}

#[cfg(feature = "native")]
#[test]
fn help_advertises_opt_in_window() {
    let output = invoke(&["--help"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("--window: interactive"));
}
