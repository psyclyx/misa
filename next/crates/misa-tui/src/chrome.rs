//! Physical terminal rows, including the local editor and its cursor.
use misa_render::Line;
use crate::Screen;
pub struct Frame { pub lines: Vec<Line>, pub cursor_row: usize, pub cursor_column: usize }
fn row(screen: &Screen, first: bool) -> Line {
    let (mode, role) = match screen.editor.mode() { crate::ed::Mode::Insert => ("┌", "mode.insert"), crate::ed::Mode::Normal => ("◆", "mode.normal") };
    let prefix = if first { format!("{mode} ") } else { "│ ".into() };
    Line { indent: 0, node: None, spans: vec![(screen.theme.role(role), misa_render::clip(&prefix, screen.width as usize)), (screen.theme.role("composer"), String::new())] }
}
pub fn composer(screen: &Screen) -> Frame {
    let width = (screen.width as usize).max(1);
    let prefix = 2.min(width.saturating_sub(1));
    let budget = width - prefix;
    let mut lines = vec![row(screen, true)];
    lines[0].spans[0].1 = misa_render::clip(&lines[0].spans[0].1, prefix);
    let mut column = 0;
    let cursor = screen.editor.split_at_cursor().0.len();
    let mut caret = (0, prefix);
    for (byte, ch) in screen.editor.text().char_indices() {
        let text = if ch == '\n' { String::new() } else if ch.is_control() { " ".into() } else { ch.to_string() };
        let text = if misa_render::width(&text) > budget { "�".into() } else { text };
        let cells = misa_render::width(&text);
        if column > 0 && column + cells > budget {
            let mut next = row(screen, false); next.spans[0].1 = misa_render::clip(&next.spans[0].1, prefix);
            lines.push(next); column = 0;
        }
        if byte == cursor { caret = (lines.len() - 1, prefix + column); }
        if ch == '\n' {
            let mut next = row(screen, false); next.spans[0].1 = misa_render::clip(&next.spans[0].1, prefix);
            lines.push(next); column = 0;
        } else { lines.last_mut().unwrap().spans[1].1.push_str(&text); column += cells; }
    }
    if cursor == screen.editor.text().len() {
        if column == budget {
            let mut next = row(screen, false); next.spans[0].1 = misa_render::clip(&next.spans[0].1, prefix);
            lines.push(next); column = 0;
        }
        caret = (lines.len() - 1, prefix + column);
    }
    let count = (screen.height as usize / 2).max(1);
    let start = caret.0.saturating_sub(count - 1);
    Frame { lines: lines.into_iter().skip(start).take(count).collect(), cursor_row: caret.0 - start, cursor_column: caret.1.min(width - 1) }
}
/// Split all chrome into physical rows so a pasted newline or a long notice
/// cannot scroll the terminal behind the retained row writer.
pub(crate) fn physical(lines: Vec<Line>, width: usize) -> Vec<Line> {
    let mut out = vec![];
    let width = width.max(1);
    for line in lines {
        let mut current = Line { indent: 0, node: line.node.clone(), spans: vec![] };
        let mut used = 0;
        for (style, text) in line.spans {
            for ch in text.chars() {
                let value = if ch == '\n' { String::new() } else if ch.is_control() { " ".into() } else { ch.to_string() };
                let value = if misa_render::width(&value) > width { "�".into() } else { value };
                let cells = misa_render::width(&value);
                if used > 0 && used + cells > width || ch == '\n' {
                    out.push(current); current = Line { indent: 0, node: line.node.clone(), spans: vec![] }; used = 0;
                }
                if ch != '\n' {
                    if let Some((last_style, last)) = current.spans.last_mut() && *last_style == style { last.push_str(&value); }
                    else { current.spans.push((style, value)); }
                    used += cells;
                }
            }
        }
        out.push(current);
    }
    out
}
pub fn frame(screen: &Screen, attachments: usize, staging: Option<&str>) -> Frame {
    let mut top = vec![];
    top.extend(screen.dialogs.lines(&screen.theme, screen.width as usize));
    if let Some(picker) = &screen.picker { top.extend(crate::picker_lines(screen, picker)); }
    if attachments > 0 { top.push(Line { indent: 0, node: None, spans: vec![(screen.theme.role("notice"), format!("{attachments} attachments · /save [1–{attachments}] <local path> · latest by default"))] }); }
    if let Some(notice) = &screen.notice { top.push(Line { indent: 0, node: None, spans: vec![(screen.theme.role("notice"), notice.clone())] }); }
    if let Some(staging) = staging { top.push(Line { indent: 0, node: None, spans: vec![(screen.theme.role("notice"), staging.into())] }); }
    top.push(Line::default());
    let mut top = physical(top, screen.width as usize);
    let input = composer(screen);
    let keep = (screen.height as usize).saturating_sub(input.lines.len());
    if top.len() > keep { top.drain(..top.len() - keep); }
    let cursor_row = top.len() + input.cursor_row;
    top.extend(input.lines);
    Frame { lines: top, cursor_row, cursor_column: input.cursor_column }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiline_wide_drafts_have_physical_rows_and_a_matching_cursor() {
        let mut screen = Screen::new(8, 10); screen.editor.set_text("日本\nhello world");
        let frame = composer(&screen);
        assert_eq!(frame.lines.iter().map(Line::text).collect::<Vec<_>>(), ["┌ 日本", "│ hello ", "│ world"]);
        assert_eq!((frame.cursor_row, frame.cursor_column), (2, 7));
        assert!(frame.lines.iter().all(|line| !line.text().contains('\n') && misa_render::width(&line.text()) <= 8));
    }
    #[test]
    fn chrome_never_exceeds_the_physical_screen() {
        let mut screen = Screen::new(20, 8); screen.editor.set_text("first\nsecond\nthird"); screen.notice = Some("a long notice that wraps onto several physical rows".into());
        let frame = frame(&screen, 5, Some("2 clipboard attachments · 1 uploading"));
        assert!(frame.lines.len() <= 8); assert!(frame.cursor_row < frame.lines.len()); assert!(frame.cursor_column < 20);
        assert!(frame.lines.iter().all(|line| !line.text().contains('\n') && misa_render::width(&line.text()) <= 20));
    }
}
