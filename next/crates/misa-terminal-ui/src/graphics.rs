//! Kitty graphics: an encoder, a decoded-image cache, and a placement planner.
//!
//! This module knows the terminal graphics protocol and image sizing, and nothing
//! about a screen, the event loop, or the retained renderer. It is a module rather
//! than methods on the renderer so a client can reuse the encoder without dragging
//! the terminal frontend along.
//!
//! # What it decides
//!
//! - The escape bytes for a kitty placement: base64 RGBA, `a=T` transmit-and-
//!   display, chunked at the protocol's payload limit, and the matching delete.
//! - How large an image should be drawn in cells: fit the available box, never
//!   enlarge past the native pixels, cap the columns to what the layout offers.
//! - Whether the terminal advertised kitty support at all.
//!
//! # What it does not decide
//!
//! Where on the screen an image goes, when to fetch it, or what to show while it
//! is missing. Those are the renderer's and the client's business; this module is
//! handed bytes and an anchor and returns bytes back.

use std::cell::RefCell;
use std::collections::HashMap;

use image::RgbaImage;

/// The kitty protocol's base64 payload limit for one escape sequence.
pub const CHUNK_BYTES: usize = 4096;

/// Rows an image may occupy in the compact view.
pub const COMPACT_ROWS: u16 = 8;

/// Rows an image may occupy when the transcript is expanded ("verbose").
pub const VERBOSE_ROWS: u16 = 24;

const APC: &str = "\x1b_G";
const ST: &str = "\x1b\\";

/// Pixel dimensions of one terminal cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

impl Default for CellSize {
    fn default() -> Self {
        // A conservative default for terminals that do not report pixel sizes.
        // The ratio is what matters for layout; the absolute value only decides
        // how many native pixels a cell is assumed to hold.
        Self {
            width: 10,
            height: 20,
        }
    }
}

impl CellSize {
    /// A cell size derived from a reported window geometry, if the terminal gave
    /// one. Returns `None` when pixels are missing or zero, which is common
    /// (notably on unix where the ioctl reports them as unused).
    pub fn from_window(
        columns: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Option<CellSize> {
        if columns == 0 || rows == 0 || pixel_width == 0 || pixel_height == 0 {
            return None;
        }
        Some(CellSize {
            width: (pixel_width as u32 / columns as u32).max(1),
            height: (pixel_height as u32 / rows as u32).max(1),
        })
    }
}

/// Where an image goes and how many cells it covers, in the units the terminal
/// addresses: `cols`/`rows` are cells, `pixel_*` are the scaled image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub cols: u16,
    pub rows: u16,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

/// One image to draw in the next frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub image_id: u32,
    pub placement_id: u32,
    /// Anchor cell, zero-based.
    pub row: u16,
    pub col: u16,
    /// Cell rows the placement covers from `row`.
    pub rows: u16,
    /// The complete kitty escape that draws it.
    pub escape: String,
}

impl Placement {
    pub fn covers(&self, row: usize) -> bool {
        let row = row as u32;
        let anchor = self.row as u32;
        row >= anchor && row < anchor + self.rows as u32
    }
}

/// Whether a terminal advertises the kitty graphics protocol.
///
/// Pure so tests need not mutate the environment. The three names are the ones
/// that ship kitty's protocol, plus an explicit override for terminals fronting
/// one of them (a multiplexer, say).
pub fn advertised(term: Option<&str>, term_program: Option<&str>, explicit: Option<&str>) -> bool {
    if explicit == Some("1") {
        return true;
    }
    [term.unwrap_or(""), term_program.unwrap_or("")]
        .iter()
        .any(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("kitty") || value.contains("ghostty") || value.contains("wezterm")
        })
}

/// Read the environment for an advertised capability. Never queries the
/// terminal synchronously; the alternative is blocking a render on a reply that
/// a terminal is not obliged to send.
pub fn detect() -> bool {
    let term = std::env::var("TERM").ok();
    let program = std::env::var("TERM_PROGRAM").ok();
    let explicit = std::env::var("MISA_KITTY_GRAPHICS").ok();
    advertised(term.as_deref(), program.as_deref(), explicit.as_deref())
}

/// Scale an image into a cell box.
///
/// The image is fit into `max_cols` columns and `max_rows` rows, preserving its
/// aspect ratio and never scaling above its native pixel size. The returned
/// `cols`/`rows` are what the layout must reserve; the returned pixel dimensions
/// are what the terminal is told to draw.
pub fn plan(
    native_width: u32,
    native_height: u32,
    cell: CellSize,
    max_cols: u16,
    max_rows: u16,
) -> Plan {
    let native_width = native_width.max(1) as u64;
    let native_height = native_height.max(1) as u64;
    let cell_width = cell.width.max(1) as u64;
    let cell_height = cell.height.max(1) as u64;
    let max_cols = max_cols.max(1) as u64;
    let max_rows = max_rows.max(1) as u64;
    let box_width = max_cols * cell_width;
    let box_height = max_rows * cell_height;
    let fit = (box_width as f64 / native_width as f64)
        .min(box_height as f64 / native_height as f64)
        .min(1.0);
    let pixel_width = ((native_width as f64 * fit).round() as u64).max(1);
    let pixel_height = ((native_height as f64 * fit).round() as u64).max(1);
    let cols = pixel_width.div_ceil(cell_width).clamp(1, max_cols) as u16;
    let rows = pixel_height.div_ceil(cell_height).clamp(1, max_rows) as u16;
    Plan {
        cols,
        rows,
        pixel_width: pixel_width as u32,
        pixel_height: pixel_height as u32,
    }
}

/// Encode an RGBA image as a chunked `a=T` transmit-and-display sequence.
///
/// `image_id` and `placement_id` name the image and the placement within it, so
/// a later delete can address exactly this one. The payload is split so no escape
/// carries more than [`CHUNK_BYTES`] of base64.
pub fn encode_transmit(image_id: u32, placement_id: u32, plan: Plan, rgba: &[u8]) -> String {
    use base64::Engine as _;
    let payload = base64::engine::general_purpose::STANDARD.encode(rgba);
    let chunks: Vec<&str> = if payload.is_empty() {
        vec![""]
    } else {
        payload
            .as_bytes()
            .chunks(CHUNK_BYTES)
            .map(|chunk| std::str::from_utf8(chunk).expect("base64 is ascii"))
            .collect()
    };
    let mut out = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let last = index + 1 == chunks.len();
        out.push_str(APC);
        if index == 0 {
            out.push_str(&format!(
                "a=T,f=100,s={},v={},c={},r={},i={},p={}",
                plan.pixel_width, plan.pixel_height, plan.cols, plan.rows, image_id, placement_id
            ));
        }
        if !last {
            out.push_str(if index == 0 { ",m=1" } else { "m=1" });
        } else if index > 0 {
            out.push_str("m=0");
        }
        out.push(';');
        out.push_str(chunk);
        out.push_str(ST);
    }
    out
}

/// Encode a delete for an image and all of its placements.
///
/// `d=i` frees the transmitted data as well: an image that is no longer shown
/// should not keep a copy in the terminal's memory.
pub fn encode_delete(image_id: u32) -> String {
    format!("{APC}a=d,d=i,i={image_id}{ST}")
}

/// The image ids the cache has handed out for a hash are stable for the life of
/// the process, so row-diffing sees an unchanged placement as unchanged.
#[derive(Default)]
struct State {
    images: HashMap<String, Cached>,
    ids: HashMap<String, u32>,
    next_id: u32,
}

struct Cached {
    rgba: RgbaImage,
    width: u32,
    height: u32,
}

/// The kitty encoder plus a cache keyed by blob hash.
///
/// Cache mutation goes through a `RefCell` because the retained renderer builds a
/// frame behind a shared `&Screen`: the client supplies decoded pixels from the
/// event loop, and the renderer reads them out without needing a mutable borrow
/// of the whole screen.
pub struct Kitty {
    enabled: bool,
    cell: CellSize,
    state: RefCell<State>,
}

impl Kitty {
    pub fn new(enabled: bool, cell: CellSize) -> Self {
        Self {
            enabled,
            cell,
            state: RefCell::new(State {
                next_id: 1,
                ..State::default()
            }),
        }
    }

    /// A cache with the terminal's advertised capability and the default cell
    /// size. The event loop supplies a measured cell size afterwards when the
    /// terminal reports one.
    pub fn detect() -> Self {
        Self::new(detect(), CellSize::default())
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn cell(&self) -> CellSize {
        self.cell
    }

    pub fn set_cell(&mut self, cell: CellSize) {
        self.cell = cell;
    }

    /// Whether decoded pixels for a hash are already cached.
    pub fn has(&self, hash: &str) -> bool {
        self.state.borrow().images.contains_key(hash)
    }

    /// The stable image id for a hash, if it has been seen.
    pub fn image_id(&self, hash: &str) -> Option<u32> {
        self.state.borrow().ids.get(hash).copied()
    }

    /// Store decoded pixels under a content hash. Decoding happens on the way in,
    /// never on the render path.
    pub fn insert(&self, hash: &str, image: RgbaImage) {
        let mut state = self.state.borrow_mut();
        let (width, height) = (image.width(), image.height());
        if !state.ids.contains_key(hash) {
            let id = state.next_id;
            state.next_id = state.next_id.wrapping_add(1).max(1);
            state.ids.insert(hash.to_string(), id);
        }
        state.images.insert(
            hash.to_string(),
            Cached {
                rgba: image,
                width,
                height,
            },
        );
    }

    /// Forget every cached image. Used when a session goes away.
    pub fn clear(&self) {
        let mut state = self.state.borrow_mut();
        state.images.clear();
        state.ids.clear();
    }

    /// The placement box for a native image at the current cell size.
    pub fn plan(
        &self,
        native_width: u32,
        native_height: u32,
        max_cols: u16,
        max_rows: u16,
    ) -> Plan {
        plan(native_width, native_height, self.cell, max_cols, max_rows)
    }

    /// The escape that draws a cached image at an anchor cell, and the rows it
    /// covers. `None` when graphics are unsupported or the pixels have not
    /// arrived yet.
    pub fn place(
        &self,
        hash: &str,
        anchor: (u16, u16),
        max_cols: u16,
        max_rows: u16,
    ) -> Option<Placement> {
        if !self.enabled {
            return None;
        }
        let state = self.state.borrow();
        let cached = state.images.get(hash)?;
        let image_id = *state.ids.get(hash)?;
        let plan = plan(cached.width, cached.height, self.cell, max_cols, max_rows);
        let escape = encode_transmit(image_id, image_id, plan, cached.rgba.as_raw());
        Some(Placement {
            image_id,
            placement_id: image_id,
            row: anchor.0,
            col: anchor.1,
            rows: plan.rows,
            escape,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell() -> CellSize {
        CellSize {
            width: 10,
            height: 20,
        }
    }

    fn rgba(width: u32, height: u32) -> Vec<u8> {
        vec![255; (width * height * 4) as usize]
    }

    fn payloads(sequence: &str) -> Vec<&str> {
        sequence
            .split(APC)
            .skip(1)
            .filter_map(|chunk| chunk.split_once(';').map(|(_, payload)| payload))
            .map(|payload| payload.strip_suffix(ST).unwrap_or(payload))
            .collect()
    }

    #[test]
    fn a_small_image_is_encoded_as_one_transmit_with_its_fields() {
        let plan = plan(4, 4, cell(), 80, COMPACT_ROWS);
        assert_eq!(
            plan,
            Plan {
                cols: 1,
                rows: 1,
                pixel_width: 4,
                pixel_height: 4,
            }
        );
        let sequence = encode_transmit(7, 9, plan, &rgba(4, 4));
        // One chunk, so no continuation marker and no `m` at all.
        assert_eq!(sequence.matches(APC).count(), 1);
        assert!(!sequence.contains("m="));
        assert!(sequence.starts_with(APC));
        assert!(sequence.ends_with(ST));
        assert!(sequence.contains("a=T"));
        assert!(sequence.contains("f=100"));
        assert!(sequence.contains("s=4"));
        assert!(sequence.contains("v=4"));
        assert!(sequence.contains("c=1"));
        assert!(sequence.contains("r=1"));
        assert!(sequence.contains("i=7"));
        assert!(sequence.contains("p=9"));
        // The payload decodes back to the pixels we handed it.
        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(payloads(&sequence)[0])
            .unwrap();
        assert_eq!(decoded, rgba(4, 4));
    }

    #[test]
    fn a_large_image_is_chunked_at_the_protocol_limit() {
        let plan = plan(100, 100, cell(), 80, VERBOSE_ROWS);
        let raw = rgba(100, 100);
        let sequence = encode_transmit(1, 1, plan, &raw);
        let chunks = payloads(&sequence);
        assert!(chunks.len() > 1, "expected a chunked payload");
        for chunk in &chunks {
            assert!(chunk.len() <= CHUNK_BYTES, "chunk over the limit");
        }
        // Only the first escape carries the control fields.
        let first = sequence.split(APC).nth(1).unwrap();
        assert!(first.contains("a=T"));
        assert!(first.contains("m=1"));
        // A middle chunk resumes with `m=1`; the last says `m=0`.
        assert!(sequence.contains(&format!("{APC}m=1;")));
        assert!(sequence.ends_with(&format!("m=0;{}{ST}", chunks.last().unwrap())));
        // The concatenated payload is exactly the image.
        use base64::Engine as _;
        let joined = chunks.join("");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(joined)
            .unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn a_delete_addresses_the_image() {
        assert_eq!(encode_delete(42), "\x1b_Ga=d,d=i,i=42\x1b\\");
        assert!(encode_delete(42).contains("a=d"));
    }

    #[test]
    fn a_wide_image_is_capped_at_the_available_columns() {
        // 4000 native pixels wide into 40 columns of 10px is 400 pixels; the
        // height follows the aspect ratio, not the pixel box.
        let plan = plan(4000, 1000, cell(), 40, VERBOSE_ROWS);
        assert_eq!(plan.cols, 40);
        assert_eq!(plan.pixel_width, 400);
        assert_eq!(plan.pixel_height, 100);
        assert_eq!(plan.rows, 5);
        assert!(plan.pixel_width <= 4000, "never upscale");
    }

    #[test]
    fn a_small_image_is_never_upscaled() {
        let plan = plan(8, 6, cell(), 80, COMPACT_ROWS);
        assert_eq!(plan.pixel_width, 8);
        assert_eq!(plan.pixel_height, 6);
        assert_eq!(plan.cols, 1);
        assert_eq!(plan.rows, 1);
    }

    #[test]
    fn a_tall_image_is_bounded_by_the_row_cap() {
        let plan = plan(100, 100_000, cell(), 80, COMPACT_ROWS);
        assert_eq!(plan.rows, COMPACT_ROWS, "rows are capped");
        // The aspect ratio still governs the width.
        assert!(plan.pixel_height <= COMPACT_ROWS as u32 * cell().height);
        assert!(plan.pixel_width < 100);
    }

    #[test]
    fn the_placement_row_count_matches_the_plan() {
        let kitty = Kitty::new(true, cell());
        kitty.insert(
            "hash",
            RgbaImage::from_raw(4000, 1000, rgba(4000, 1000)).unwrap(),
        );
        let placement = kitty.place("hash", (3, 1), 40, VERBOSE_ROWS).unwrap();
        let plan = kitty.plan(4000, 1000, 40, VERBOSE_ROWS);
        assert_eq!(placement.rows, plan.rows);
        assert_eq!(placement.row, 3);
        assert_eq!(placement.col, 1);
        assert_eq!(placement.covers(3), true);
        assert_eq!(placement.covers(3 + plan.rows as usize - 1), true);
        assert_eq!(placement.covers(3 + plan.rows as usize), false);
    }

    #[test]
    fn an_unsupported_terminal_keeps_the_placeholder() {
        let kitty = Kitty::new(false, cell());
        kitty.insert("hash", RgbaImage::from_raw(4, 4, rgba(4, 4)).unwrap());
        assert!(kitty.has("hash"));
        assert!(kitty.place("hash", (0, 0), 80, COMPACT_ROWS).is_none());
    }

    #[test]
    fn capability_detection_recognises_the_protocol_terminals() {
        assert!(advertised(Some("xterm-kitty"), None, None));
        assert!(advertised(None, Some("WezTerm"), None));
        assert!(advertised(Some("xterm-ghostty"), None, None));
        assert!(advertised(None, Some("ghostty"), None));
        assert!(advertised(None, None, Some("1")));
        assert!(!advertised(
            Some("xterm-256color"),
            Some("Apple_Terminal"),
            None
        ));
        assert!(!advertised(Some("screen"), Some("tmux"), Some("0")));
        assert!(!advertised(None, None, None));
    }

    #[test]
    fn a_missing_image_has_no_placement_until_it_arrives() {
        let kitty = Kitty::new(true, cell());
        assert!(kitty.place("missing", (0, 0), 80, COMPACT_ROWS).is_none());
        kitty.insert("missing", RgbaImage::from_raw(2, 2, rgba(2, 2)).unwrap());
        assert!(kitty.place("missing", (0, 0), 80, COMPACT_ROWS).is_some());
    }

    #[test]
    fn a_cell_size_is_derived_only_from_a_real_window_geometry() {
        assert_eq!(
            CellSize::from_window(100, 40, 1000, 800),
            Some(CellSize {
                width: 10,
                height: 20
            })
        );
        assert_eq!(CellSize::from_window(0, 40, 1000, 800), None);
        assert_eq!(CellSize::from_window(100, 40, 0, 800), None);
    }
}
