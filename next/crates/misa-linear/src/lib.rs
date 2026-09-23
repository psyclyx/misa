//! Client rendering that is not terminal-specific.
//!
//! This crate owns the two pieces of `misa-tui` that never needed the terminal
//! frontend:
//!
//! - [`Live`], the incremental streaming renderer: it parses the in-flight
//!   markdown, lays out only the blocks an append changed, and exposes the same
//!   [`misa_lines::Line`] rows a settled body renders to.
//! - [`graphics`], the kitty encoder, decoded-image cache, and placement planner.
//!   It turns decoded pixels and an anchor into the escape bytes a terminal
//!   draws, and nothing here knows about a screen or an event loop.
//!
//! Both speak the neutral vocabulary of the middle layer, so a client whose
//! medium is lines can render a session without dragging a terminal along.

pub mod graphics;
mod live;

pub use live::{Live, Paint, THINKING_TAIL_LINES, stream_order, thinking_stream};
