//! Semantic keyboard routing and frame adapter for terminal hosts.
use std::io::Write;

/// Shared keyboard path after terminal translation and any modal dialog.
/// The semantic panel owns input before a retained selection; then the
/// client's editor/picker handles the key.
pub fn route_key(
    screen: &mut crate::Screen,
    retained: &mut crate::retained::Retained,
    view: &misa_proto::Node,
    key: crate::Key,
) -> crate::KeyOut {
    screen
        .panel_key(view, &key)
        .or_else(|| retained.selection_key(screen, &key))
        .unwrap_or_else(|| screen.key(key))
}

pub fn paint(
    writer: &mut impl Write,
    output: &mut misa_terminal_ui::output::Output,
    screen: &crate::Screen,
    frame: crate::chrome::Frame,
) -> Result<(), String> {
    output
        .paint(writer, &frame.lines, screen.width as usize, &frame.images)
        .map_err(|e| e.to_string())?;
    write!(
        writer,
        "\x1b[{};{}H",
        frame.cursor_row + 1,
        frame.cursor_column + 1
    )
    .and_then(|_| writer.flush())
    .map_err(|e| e.to_string())
}
