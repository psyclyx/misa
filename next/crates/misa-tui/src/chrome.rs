//! Physical terminal rows, including the local editor and its cursor.
use crate::Screen;
use misa_render::Line;
pub struct Frame {
    pub lines: Vec<Line>,
    pub cursor_row: usize,
    pub cursor_column: usize,
}
fn row(screen: &Screen, first: bool) -> Line {
    let (mode, role) = match screen.editor.mode() {
        crate::ed::Mode::Insert => ("│", "mode.insert"),
        crate::ed::Mode::Normal => ("◆", "mode.normal"),
        crate::ed::Mode::Visual => ("◇", "mode.visual"),
    };
    let prefix = if first {
        format!("{mode} ")
    } else {
        "│ ".into()
    };
    Line {
        surface: None,
        indent: 0,
        node: None,
        spans: vec![
            (
                screen.theme.role(role),
                misa_render::clip(&prefix, screen.width as usize),
            ),
            (screen.theme.role("composer"), String::new()),
        ],
    }
}
fn text_style(screen: &Screen, byte: usize) -> misa_render::Style {
    let base = screen.theme.role("user");
    let selected = screen
        .editor
        .visual_range()
        .is_some_and(|(start, end)| byte >= start && byte < end);
    if selected {
        base.over(screen.theme.role("selection"))
    } else {
        base
    }
}
fn push_text(line: &mut Line, style: misa_render::Style, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some((last_style, last)) = line.spans.last_mut()
        && *last_style == style
    {
        last.push_str(text);
    } else {
        line.spans.push((style, text.to_string()));
    }
}
pub fn composer(screen: &Screen) -> Frame {
    composer_with_budget(screen, (screen.height as usize / 2).max(1))
}

/// Lay out the local editor in the rows the frame policy actually reserved for it.
///
/// The editor owns its cursor-centred viewport; the surrounding frame owns how many
/// rows it may consume. Keeping that distinction here prevents the retained renderer
/// from having to reconstruct cursor coordinates after clipping an already-laid-out
/// composer.
pub(crate) fn composer_with_budget(screen: &Screen, max_rows: usize) -> Frame {
    let width = (screen.width as usize).max(1);
    let prefix = 2.min(width.saturating_sub(1));
    let budget = width - prefix;
    let mut lines = vec![row(screen, true)];
    lines[0].spans[0].1 = misa_render::clip(&lines[0].spans[0].1, prefix);
    let mut column = 0;
    let cursor = screen.editor.split_at_cursor().0.len();
    let mut caret = (0, prefix);
    for (byte, ch) in screen.editor.text().char_indices() {
        let text = if ch == '\n' {
            String::new()
        } else if ch.is_control() {
            " ".into()
        } else {
            ch.to_string()
        };
        let text = if misa_render::width(&text) > budget {
            "�".into()
        } else {
            text
        };
        let cells = misa_render::width(&text);
        if column > 0 && column + cells > budget {
            let mut next = row(screen, false);
            next.spans[0].1 = misa_render::clip(&next.spans[0].1, prefix);
            lines.push(next);
            column = 0;
        }
        if byte == cursor {
            caret = (lines.len() - 1, prefix + column);
        }
        if ch == '\n' {
            let mut next = row(screen, false);
            next.spans[0].1 = misa_render::clip(&next.spans[0].1, prefix);
            lines.push(next);
            column = 0;
        } else {
            let style = text_style(screen, byte);
            push_text(lines.last_mut().unwrap(), style, &text);
            column += cells;
        }
    }
    if cursor == screen.editor.text().len() {
        if column == budget {
            let mut next = row(screen, false);
            next.spans[0].1 = misa_render::clip(&next.spans[0].1, prefix);
            lines.push(next);
            column = 0;
        }
        caret = (lines.len() - 1, prefix + column);
    }
    let count = max_rows.max(1);
    let start = caret.0.saturating_sub(count - 1);
    Frame {
        lines: lines.into_iter().skip(start).take(count).collect(),
        cursor_row: caret.0 - start,
        cursor_column: caret.1.min(width - 1),
    }
}
pub(crate) fn header(screen: &Screen) -> Vec<Line> {
    vec![
        Line {
            surface: None,
            indent: 0,
            node: None,
            spans: vec![
                (screen.theme.role("header.title"), "misa".into()),
                (
                    screen.theme.role("header.detail"),
                    "  ·  coding agent".into(),
                ),
            ],
        },
        Line {
            surface: None,
            indent: 0,
            node: None,
            spans: vec![(
                screen.theme.role("header.detail"),
                "/ conversation   : actions   F1 help".into(),
            )],
        },
    ]
}
/// Split all chrome into physical rows so a pasted newline or a long notice
/// cannot scroll the terminal behind the retained row writer.
pub(crate) fn physical(lines: Vec<Line>, width: usize) -> Vec<Line> {
    let mut out = vec![];
    let width = width.max(1);
    for line in lines {
        let mut current = Line {
            surface: line.surface,
            indent: 0,
            node: line.node.clone(),
            spans: vec![],
        };
        let mut used = 0;
        for (style, text) in line.spans {
            for ch in text.chars() {
                let value = if ch == '\n' {
                    String::new()
                } else if ch.is_control() {
                    " ".into()
                } else {
                    ch.to_string()
                };
                let value = if misa_render::width(&value) > width {
                    "�".into()
                } else {
                    value
                };
                let cells = misa_render::width(&value);
                if used > 0 && used + cells > width || ch == '\n' {
                    out.push(current);
                    current = Line {
                        surface: line.surface,
                        indent: 0,
                        node: line.node.clone(),
                        spans: vec![],
                    };
                    used = 0;
                }
                if ch != '\n' {
                    if let Some((last_style, last)) = current.spans.last_mut()
                        && *last_style == style
                    {
                        last.push_str(&value);
                    } else {
                        current.spans.push((style, value));
                    }
                    used += cells;
                }
            }
        }
        out.push(current);
    }
    out
}
pub(crate) fn top(screen: &Screen, _attachments: usize, _staging: Option<&str>) -> Vec<Line> {
    // The session document is shared with other clients. Application chrome is
    // terminal-local, so the root never has to carry this header as content.
    // The reference suppresses header/status chrome on terminals too short to give
    // the document a useful room. This is a frame policy, not a terminal renderer
    // accident, so every retained layout gets the same answer.
    let mut top = if screen.height >= 4 {
        header(screen)
    } else {
        Vec::new()
    };
    // A focused request/report is an exclusive layer. Non-focused requests still
    // advertise themselves in normal chrome and cannot steal the composer.
    if !screen.dialogs.modal() {
        top.extend(screen.dialogs.lines(
            &screen.theme,
            screen.width as usize,
            &screen.prefs.dialogs,
        ));
    }
    if let Some(notice) = &screen.notice {
        top.push(Line {
            surface: None,
            indent: 0,
            node: None,
            spans: vec![(screen.theme.role("notice"), notice.clone())],
        });
    }
    physical(top, screen.width as usize)
}

#[cfg(test)]
pub fn frame(screen: &Screen, attachments: usize, staging: Option<&str>) -> Frame {
    let mut top = top(screen, attachments, staging);
    if screen.dialogs.modal() {
        let height = screen.height as usize;
        top.truncate(height);
        let available = height.saturating_sub(top.len());
        let dialog = physical(
            screen.dialogs.lines(
                &screen.theme,
                screen.width as usize,
                &screen.prefs.dialogs,
            ),
            screen.width as usize,
        );
        let take = dialog.len().min(available);
        let mut lines = top;
        lines.extend(dialog.into_iter().take(take));
        return Frame {
            cursor_row: lines.len().saturating_sub(1),
            cursor_column: 0,
            lines,
        };
    }
    let overlay = screen
        .picker
        .as_ref()
        .is_some_and(misa_kit::picker::Picker::is_overlay);
    if !overlay {
        top.push(Line::default());
    }
    let input = composer(screen);
    let picker = screen
        .picker
        .as_ref()
        .filter(|picker| picker.is_overlay())
        .map(|picker| {
            let mut lines = physical(crate::picker_lines(screen, picker), screen.width as usize);
            // The picker owns the available region, but the region can be shorter
            // than its preferred height. Keep the query/title row visible and let
            // the layout, rather than the picker state, decide what fits.
            let budget = (screen.height as usize).saturating_sub(top.len()).max(1);
            lines.truncate(budget);
            lines
        });
    // The standalone frame is used by small clients and tests without a transcript
    // owner. It still keeps the picker as the focused input; the retained terminal
    // inserts the same picker into the reference overlay region below.
    let completions = screen
        .picker
        .as_ref()
        .filter(|picker| picker.is_inline())
        .map(|picker| crate::completion_lines(screen, picker))
        .unwrap_or_default();
    let (reserved, cursor) = if let Some(picker) = &picker {
        let query = screen
            .picker
            .as_ref()
            .map(|picker| {
                let prefix = match picker.accept {
                    misa_kit::picker::Accept::Run => "/",
                    misa_kit::picker::Accept::Action { .. } => ":",
                    misa_kit::picker::Accept::Argument { .. } => "",
                };
                format!("  {}: {}{}", picker.title, prefix, picker.query)
            })
            .unwrap_or_default();
        let column = misa_render::width(&query).min(screen.width.saturating_sub(1) as usize);
        (picker.len(), (0usize, column))
    } else {
        (
            input.lines.len() + completions.len(),
            (input.cursor_row, input.cursor_column),
        )
    };
    let keep = (screen.height as usize).saturating_sub(reserved);
    if top.len() > keep {
        top.drain(..top.len() - keep);
    }
    let cursor_row = top.len() + cursor.0;
    if let Some(picker) = picker {
        top.extend(picker);
    } else {
        top.extend(input.lines);
        top.extend(completions);
    }
    Frame {
        lines: top,
        cursor_row,
        cursor_column: cursor.1,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiline_wide_drafts_have_physical_rows_and_a_matching_cursor() {
        let mut screen = Screen::new(8, 10);
        screen.editor.set_text("日本\nhello world");
        let frame = composer(&screen);
        assert_eq!(
            frame.lines.iter().map(Line::text).collect::<Vec<_>>(),
            ["│ 日本", "│ hello ", "│ world"]
        );
        assert_eq!((frame.cursor_row, frame.cursor_column), (2, 7));
        assert!(
            frame
                .lines
                .iter()
                .all(|line| !line.text().contains('\n') && misa_render::width(&line.text()) <= 8)
        );
    }
    #[test]
    fn chrome_never_exceeds_the_physical_screen() {
        let mut screen = Screen::new(20, 8);
        screen.editor.set_text("first\nsecond\nthird");
        screen.notice = Some("a long notice that wraps onto several physical rows".into());
        let frame = frame(&screen, 5, Some("2 clipboard attachments · 1 uploading"));
        assert!(frame.lines.len() <= 8);
        assert!(frame.cursor_row < frame.lines.len());
        assert!(frame.cursor_column < 20);
        assert!(
            frame
                .lines
                .iter()
                .all(|line| !line.text().contains('\n') && misa_render::width(&line.text()) <= 20)
        );
    }
}
