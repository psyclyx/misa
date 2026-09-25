//! Local physical rows only: no semantic document, client, or connected screen.
use misa_style::{Color, Style};
use misa_terminal_ui::{
    StyledRow,
    output::Output,
    viewport::{Head, Request, Viewport},
};
use std::io;

#[derive(Clone)]
struct Item {
    id: u64,
    text: String,
}

struct Fixture {
    items: Vec<Item>,
    next_id: u64,
    width: usize,
    height: usize,
    scroll: usize,
    intent: u64,
    follow: bool,
    selected: Option<u64>,
    viewport: Viewport<u64, u64>,
    painter: Output,
}

impl Fixture {
    fn new(width: usize, height: usize) -> Self {
        Self {
            items: Vec::new(),
            next_id: 1,
            width,
            height,
            scroll: 0,
            intent: 0,
            follow: true,
            selected: None,
            viewport: Viewport::new(true),
            painter: Output::default(),
        }
    }

    fn insert(&mut self, index: usize, text: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.items.insert(
            index,
            Item {
                id,
                text: text.into(),
            },
        );
        self.viewport.layout_changed();
        id
    }

    fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.viewport.layout_changed();
    }

    // Input is normalized here: the viewport sees an intent, never a terminal event.
    fn scroll_to(&mut self, row: usize) {
        self.scroll = row;
        self.intent += 1;
        self.follow = false;
    }

    fn select(&mut self, id: u64) {
        self.selected = Some(id);
    }

    fn rows(&self) -> Vec<(u64, StyledRow)> {
        let mut rows = Vec::new();
        let width = self.width.max(1);
        for item in &self.items {
            // ASCII fixture text: split by display column, with at least one row
            // for an empty item. A wrapped item retains one local key across rows.
            let chunks: Vec<String> = if item.text.is_empty() {
                vec![String::new()]
            } else {
                item.text
                    .as_bytes()
                    .chunks(width)
                    .map(|chunk| String::from_utf8(chunk.to_vec()).expect("ASCII fixture"))
                    .collect()
            };
            for chunk in chunks {
                let style = if self.selected == Some(item.id) {
                    Style::fg(Color::Indexed(11)).bold()
                } else {
                    Style::fg(Color::Indexed(7))
                };
                rows.push((
                    item.id,
                    StyledRow {
                        spans: vec![(style, chunk)],
                        ..StyledRow::default()
                    },
                ));
            }
        }
        rows
    }

    fn frame(&mut self) -> io::Result<Vec<u8>> {
        let rows = self.rows();
        let head = self.selected.and_then(|id| {
            rows.iter()
                .position(|(key, _)| *key == id)
                .map(|row| Head { identity: id, row })
        });
        let first = self.viewport.resolve(
            Request {
                scroll: self.scroll,
                follow: self.follow,
                intent: self.intent,
                room: self.height,
                head,
            },
            rows.len(),
            |row| rows.get(row).map(|(key, _)| *key),
            |key, offset| {
                rows.iter()
                    .position(|(id, _)| id == key)
                    .map(|row| row + offset)
            },
        );
        self.scroll = first;
        self.follow = self.viewport.following();
        let visible: Vec<StyledRow> = rows
            .iter()
            .skip(first)
            .take(self.height)
            .map(|(_, row)| row.clone())
            .collect();
        let mut bytes = Vec::new();
        self.painter.paint(&mut bytes, &visible, self.width, &[])?;
        Ok(bytes)
    }
}

// The executable is also an assertion-bearing, deterministic headless demonstration.
fn demo() -> io::Result<()> {
    let mut fixture = Fixture::new(12, 2);
    fixture.insert(0, "alpha");
    let anchor = fixture.insert(1, "bravo");
    fixture.insert(2, "charlie");
    let initial = fixture.frame()?;
    assert!(initial.starts_with(b"\x1b[2J"));
    assert!(
        fixture.frame()?.is_empty(),
        "unchanged frame must be silent"
    );
    fixture.scroll_to(1);
    assert!(
        fixture.frame()?.is_empty(),
        "scroll intent can anchor an already visible frame"
    );
    fixture.insert(0, "inserted");
    let shifted = fixture.frame()?;
    assert_eq!(
        fixture.scroll, 2,
        "local key anchors reader across insertion"
    );
    assert!(
        shifted.is_empty(),
        "insertion above anchor needs no repaint"
    );
    fixture.select(anchor);
    let selection = String::from_utf8(fixture.frame()?).unwrap();
    assert!(selection.contains("\x1b[1;1H") && selection.contains("bravo"));
    assert!(
        !selection.contains("charlie"),
        "selection repaints only changed row"
    );
    fixture.scroll_to(usize::MAX);
    fixture.frame()?;
    assert!(fixture.follow);
    fixture.insert(fixture.items.len(), "delta");
    let appended = String::from_utf8(fixture.frame()?).unwrap();
    assert!(appended.contains("delta"));
    assert_eq!(fixture.scroll, 3);
    fixture.resize(3, 1);
    let tiny = String::from_utf8(fixture.frame()?).unwrap();
    assert!(
        tiny.contains("\x1b[2J"),
        "width invalidates retained painter"
    );
    assert_eq!(fixture.rows().len(), 12);
    assert_eq!(fixture.scroll, 11);
    println!(
        "misa-terminal-testbed: headless ANSI/diff, anchor, selection, follow, resize and tiny viewport OK"
    );
    Ok(())
}

fn main() -> io::Result<()> {
    demo()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_fixture() {
        demo().unwrap();
    }

    #[test]
    fn wrapped_anchor_and_selection_reveal() {
        let mut f = Fixture::new(3, 1);
        f.insert(0, "aaa");
        let key = f.insert(1, "abcdef");
        f.insert(2, "zzz");
        f.scroll_to(2);
        let first = String::from_utf8(f.frame().unwrap()).unwrap();
        assert!(first.contains("def"));
        f.insert(0, "xxx");
        assert!(f.frame().unwrap().is_empty());
        assert_eq!(f.scroll, 3, "second wrapped row remains anchored");
        f.select(key);
        let selected = String::from_utf8(f.frame().unwrap()).unwrap();
        assert_eq!(f.scroll, 2, "new selection reveals first row of keyed item");
        assert!(selected.contains("abc"));
        assert!(selected.contains("\x1b[1;1H"));
    }
}
