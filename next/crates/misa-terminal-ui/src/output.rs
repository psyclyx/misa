//! Retained terminal rows: unchanged rows produce no terminal bytes.
use crate::graphics::{self, Placement};
use crate::terminal_style::sgr;
use crate::{PhysicalRow, StyledRow};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub struct Output {
    previous: Vec<StyledRow>,
    initialized: bool,
    width: usize,
    /// The kitty placements drawn by the last frame, so this frame can delete the
    /// ones that moved or went away.
    placements: Vec<Placement>,
}

impl Output {
    pub fn invalidate(&mut self) {
        self.initialized = false;
    }

    /// Paint a frame. Text rows keep the retained-row diff; image rows are treated
    /// as opaque and are re-emitted only when their placement changes.
    pub fn paint<R: PhysicalRow>(
        &mut self,
        writer: &mut impl Write,
        lines: &[R],
        width: usize,
        images: &[Placement],
    ) -> std::io::Result<()> {
        let width = width.max(1);
        if self.width != width {
            self.width = width;
            self.invalidate();
        }
        if !self.initialized {
            // Erasing the screen does not erase kitty graphics, so delete the
            // placements explicitly before clearing the text.
            for placement in &self.placements {
                write!(writer, "{}", graphics::encode_delete(placement.image_id))?;
            }
            write!(writer, "\x1b[2J")?;
            self.previous.clear();
            self.placements.clear();
        }
        let current: HashMap<u32, &Placement> = images
            .iter()
            .map(|placement| (placement.image_id, placement))
            .collect();
        let image_rows: HashSet<usize> = images
            .iter()
            .flat_map(|placement| {
                (placement.row..placement.row.saturating_add(placement.rows))
                    .map(|row| row as usize)
            })
            .collect();
        let previous_image_rows: HashSet<usize> = self
            .placements
            .iter()
            .flat_map(|placement| {
                (placement.row..placement.row.saturating_add(placement.rows))
                    .map(|row| row as usize)
            })
            .collect();
        // Delete before redrawing so a moved image never covers the text on both
        // its old and new rows at once.
        for old in &self.placements {
            let unchanged = current.get(&old.image_id).is_some_and(|new| {
                new.row == old.row && new.col == old.col && new.escape == old.escape
            });
            if !unchanged {
                write!(writer, "{}", graphics::encode_delete(old.image_id))?;
            }
        }
        for (row, line) in lines.iter().enumerate() {
            if image_rows.contains(&row) {
                // The image covers this row; its placeholder text is not written.
                continue;
            }
            let old = self.previous.get(row);
            let was_image = previous_image_rows.contains(&row);
            if !was_image && old.is_some_and(|old| old.matches(line)) {
                continue;
            }
            // A growing final span can be sent as its suffix. Styled prefixes and
            // indentation must match; edits elsewhere replace only this row. A row
            // that used to hold an image never takes the suffix path: the image
            // must be fully overwritten.
            let suffix = (!was_image)
                .then(|| old.and_then(|old| append_suffix(old, line)))
                .flatten();
            if let Some((column, style, text)) = suffix {
                write!(
                    writer,
                    "\x1b[{};{}H{}{}\x1b[0m",
                    row + 1,
                    column + 1,
                    sgr(&style),
                    text
                )?;
            } else {
                write!(writer, "\x1b[{};1H\x1b[2K", row + 1)?;
                if let Some(surface) = line.surface() {
                    write!(writer, "{}", sgr(&surface))?;
                }
                write!(writer, "{}", " ".repeat(line.indent() as usize))?;
                for (style, text) in line.spans() {
                    write!(
                        writer,
                        "{}{}\x1b[0m",
                        sgr(&style.over(line.surface().unwrap_or(misa_style::Style::PLAIN))),
                        text
                    )?;
                }
                if let Some(surface) = line.surface() {
                    let used = line.indent() as usize
                        + line
                            .spans()
                            .iter()
                            .map(|(_, text)| UnicodeWidthStr::width(text.as_str()))
                            .sum::<usize>();
                    let fill = width.saturating_sub(used);
                    if fill > 0 {
                        write!(writer, "{}{}\x1b[0m", sgr(&surface), " ".repeat(fill))?;
                    }
                }
            }
        }
        // Emit only placements that are new or moved; an unchanged image is
        // already on screen and the terminal keeps it.
        for placement in images {
            let unchanged = self.placements.iter().any(|old| {
                old.image_id == placement.image_id
                    && old.row == placement.row
                    && old.col == placement.col
                    && old.escape == placement.escape
            });
            if !unchanged {
                write!(
                    writer,
                    "\x1b[{};{}H{}",
                    placement.row as usize + 1,
                    placement.col as usize + 1,
                    placement.escape
                )?;
            }
        }
        if self.previous.len() > lines.len() {
            write!(writer, "\x1b[{};1H\x1b[J", lines.len() + 1)?;
        }
        writer.flush()?;
        // Retain only physical content. Unchanged rows keep their allocation;
        // changing a node id in a semantic caller cannot dirty a row.
        self.previous.truncate(lines.len());
        for (index, line) in lines.iter().enumerate() {
            match self.previous.get_mut(index) {
                Some(old) if !old.matches(line) => *old = StyledRow::from_row(line),
                None => self.previous.push(StyledRow::from_row(line)),
                _ => {}
            }
        }
        self.placements = images.to_vec();
        self.initialized = true;
        Ok(())
    }
}

fn append_suffix<'a>(
    old: &StyledRow,
    new: &'a impl PhysicalRow,
) -> Option<(usize, misa_style::Style, &'a str)> {
    if old.indent != new.indent()
        || old.surface != new.surface()
        || old.spans.len() != new.spans().len()
    {
        return None;
    }
    let (old_last, old_prefix) = old.spans.split_last()?;
    let (new_last, new_prefix) = new.spans().split_last()?;
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
            .map(|(_, text)| UnicodeWidthStr::width(text.as_str()))
            .sum::<usize>();
    Some((
        column,
        new_last
            .0
            .over(new.surface().unwrap_or(misa_style::Style::PLAIN)),
        suffix,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line(text: &str) -> StyledRow {
        StyledRow {
            spans: vec![(misa_style::Style::PLAIN, text.into())],
            ..StyledRow::default()
        }
    }
    #[test]
    fn unchanged_rows_are_silent_and_stream_growth_only_sends_the_suffix() {
        let mut output = Output::default();
        let mut bytes = Vec::new();
        output
            .paint(&mut bytes, &[line("history"), line("é")], 20, &[])
            .unwrap();
        bytes.clear();
        output
            .paint(&mut bytes, &[line("history"), line("é")], 20, &[])
            .unwrap();
        assert!(bytes.is_empty());
        output
            .paint(&mut bytes, &[line("history"), line("é🙂")], 20, &[])
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\x1b[2;2H"));
        assert!(text.contains("🙂"));
        assert!(!text.contains("history") && !text.contains("é") && !text.contains("[2J"));
    }
    #[test]
    fn changing_only_a_surface_repaints_and_unchanged_rows_keep_their_spans() {
        let mut output = Output::default();
        let mut bytes = Vec::new();
        let row = line("é🙂");
        output.paint(&mut bytes, &[row.clone()], 8, &[]).unwrap();
        let span_ptr = output.previous[0].spans[0].1.as_ptr();
        bytes.clear();
        output.paint(&mut bytes, &[row.clone()], 8, &[]).unwrap();
        assert!(bytes.is_empty());
        assert_eq!(span_ptr, output.previous[0].spans[0].1.as_ptr());
        let painted = StyledRow {
            surface: Some(misa_style::Style::PLAIN.bold()),
            ..row
        };
        output.paint(&mut bytes, &[painted], 8, &[]).unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains("\u{1b}[1m"));
    }

    #[test]
    fn a_shorter_frame_erases_stale_rows_without_clearing_the_screen() {
        let mut output = Output::default();
        let mut bytes = Vec::new();
        output
            .paint(&mut bytes, &[line("kept"), line("removed")], 20, &[])
            .unwrap();
        bytes.clear();
        output.paint(&mut bytes, &[line("kept")], 20, &[]).unwrap();
        assert_eq!(bytes, b"\x1b[2;1H\x1b[J");
    }
    #[test]
    fn a_moved_image_is_deleted_and_redrawn_once() {
        let placement = |row: u16| Placement {
            image_id: 5,
            placement_id: 5,
            row,
            col: 0,
            rows: 1,
            escape: format!("<image at {row}>"),
        };
        let mut output = Output::default();
        let mut bytes = Vec::new();
        let lines = [line("a"), line("b")];
        output
            .paint(&mut bytes, &lines, 20, &[placement(0)])
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("<image at 0>"));
        bytes.clear();
        // The same placement is silent.
        output
            .paint(&mut bytes, &lines, 20, &[placement(0)])
            .unwrap();
        assert!(bytes.is_empty());
        // Moving it deletes the old image, repaints both text rows, and draws anew.
        output
            .paint(&mut bytes, &lines, 20, &[placement(1)])
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("a=d,d=i,i=5"), "old placement deleted");
        assert!(text.contains("<image at 1>"), "new placement drawn");
        assert!(!text.contains("<image at 0>"));
    }
}
