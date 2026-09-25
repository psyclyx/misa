//! Physical terminal rendering from styled rows, independent of semantic documents.
pub mod clipboard;
pub mod graphics;
pub mod output;
pub mod terminal_style;
pub mod viewport;

use misa_style::Style;

/// The physical content of a terminal row. Semantic identity is intentionally absent.
pub trait PhysicalRow {
    fn indent(&self) -> u8;
    fn spans(&self) -> &[(Style, String)];
    fn surface(&self) -> Option<Style>;
}

/// Owned physical row for clients without a semantic line renderer, and for the
/// painter's retained cache. Only these fields can affect terminal bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyledRow {
    pub indent: u8,
    pub spans: Vec<(Style, String)>,
    pub surface: Option<Style>,
}

impl PhysicalRow for StyledRow {
    fn indent(&self) -> u8 {
        self.indent
    }
    fn spans(&self) -> &[(Style, String)] {
        &self.spans
    }
    fn surface(&self) -> Option<Style> {
        self.surface
    }
}

impl StyledRow {
    fn from_row(row: &impl PhysicalRow) -> Self {
        Self {
            indent: row.indent(),
            spans: row.spans().to_vec(),
            surface: row.surface(),
        }
    }

    fn matches(&self, row: &impl PhysicalRow) -> bool {
        self.indent == row.indent() && self.surface == row.surface() && self.spans == row.spans()
    }
}
