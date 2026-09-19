//! Retained terminal rows: unchanged rows produce no terminal bytes.
use misa_render::Line;
use std::io::Write;

#[derive(Default)]
pub struct Output {
    previous: Vec<Line>,
    initialized: bool,
    width: usize,
}

impl Output {
    pub fn invalidate(&mut self) {
        self.initialized = false;
    }

    pub fn paint(
        &mut self,
        writer: &mut impl Write,
        lines: &[Line],
        width: usize,
    ) -> std::io::Result<()> {
        let width = width.max(1);
        if self.width != width {
            self.width = width;
            self.invalidate();
        }
        if !self.initialized {
            write!(writer, "\x1b[2J")?;
            self.previous.clear();
        }
        for (row, line) in lines.iter().enumerate() {
            let old = self.previous.get(row);
            if old == Some(line) {
                continue;
            }
            // A growing final span can be sent as its suffix. Styled prefixes and
            // indentation must match; edits elsewhere replace only this row.
            let suffix = old.and_then(|old| append_suffix(old, line));
            if let Some((column, style, text)) = suffix {
                write!(
                    writer,
                    "\x1b[{};{}H{}{}\x1b[0m",
                    row + 1,
                    column + 1,
                    crate::sgr(&style),
                    text
                )?;
            } else {
                write!(writer, "\x1b[{};1H\x1b[2K", row + 1)?;
                if let Some(surface) = line.surface {
                    write!(writer, "{}", crate::sgr(&surface))?;
                }
                write!(writer, "{}", " ".repeat(line.indent as usize))?;
                for (style, text) in &line.spans {
                    write!(
                        writer,
                        "{}{}\x1b[0m",
                        crate::sgr(&style.over(line.surface.unwrap_or(misa_render::Style::PLAIN))),
                        text
                    )?;
                }
                if let Some(surface) = line.surface {
                    let used = line.indent as usize
                        + line
                            .spans
                            .iter()
                            .map(|(_, text)| misa_render::width(text))
                            .sum::<usize>();
                    let fill = width.saturating_sub(used);
                    if fill > 0 {
                        write!(
                            writer,
                            "{}{}\x1b[0m",
                            crate::sgr(&surface),
                            " ".repeat(fill)
                        )?;
                    }
                }
            }
        }
        if self.previous.len() > lines.len() {
            write!(writer, "\x1b[{};1H\x1b[J", lines.len() + 1)?;
        }
        writer.flush()?;
        self.previous = lines.to_vec();
        self.initialized = true;
        Ok(())
    }
}

fn append_suffix<'a>(old: &Line, new: &'a Line) -> Option<(usize, misa_render::Style, &'a str)> {
    if old.indent != new.indent || old.surface != new.surface || old.spans.len() != new.spans.len()
    {
        return None;
    }
    let (old_last, old_prefix) = old.spans.split_last()?;
    let (new_last, new_prefix) = new.spans.split_last()?;
    if old_prefix != new_prefix || old_last.0 != new_last.0 {
        return None;
    }
    let suffix = new_last.1.strip_prefix(&old_last.1)?;
    if suffix.is_empty() {
        return None;
    }
    let column = old.indent as usize
        + old
            .spans
            .iter()
            .map(|(_, text)| misa_render::width(text))
            .sum::<usize>();
    Some((
        column,
        new_last
            .0
            .over(new.surface.unwrap_or(misa_render::Style::PLAIN)),
        suffix,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line(text: &str) -> Line {
        Line {
            spans: vec![(misa_render::Theme::plain().role("text"), text.into())],
            ..Line::default()
        }
    }
    #[test]
    fn unchanged_rows_are_silent_and_stream_growth_only_sends_the_suffix() {
        let mut output = Output::default();
        let mut bytes = Vec::new();
        output
            .paint(&mut bytes, &[line("history"), line("é")], 20)
            .unwrap();
        bytes.clear();
        output
            .paint(&mut bytes, &[line("history"), line("é")], 20)
            .unwrap();
        assert!(bytes.is_empty());
        output
            .paint(&mut bytes, &[line("history"), line("é🙂")], 20)
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\x1b[2;2H"));
        assert!(text.contains("🙂"));
        assert!(!text.contains("history") && !text.contains("é") && !text.contains("[2J"));
    }
    #[test]
    fn a_shorter_frame_erases_stale_rows_without_clearing_the_screen() {
        let mut output = Output::default();
        let mut bytes = Vec::new();
        output
            .paint(&mut bytes, &[line("kept"), line("removed")], 20)
            .unwrap();
        bytes.clear();
        output.paint(&mut bytes, &[line("kept")], 20).unwrap();
        assert_eq!(bytes, b"\x1b[2;1H\x1b[J");
    }
}
