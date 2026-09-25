//! Client rendering that is not terminal-specific.
//!
//! This crate owns the two pieces of `misa-tui` that never needed the terminal
//! frontend:
//!
//! - [`Live`], the incremental streaming renderer: it parses the in-flight
//!   markdown, lays out only the blocks an append changed, and exposes the same
//!   [`misa_lines::Line`] rows a settled body renders to.
//! This crate speaks the neutral vocabulary of styled lines; physical terminal
//! encoding lives in `misa-terminal-ui`.

mod live;

pub use live::{Live, Paint, THINKING_TAIL_LINES, stream_order, thinking_stream};
