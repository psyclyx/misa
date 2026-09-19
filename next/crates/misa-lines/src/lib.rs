//! The linear renderer: the semantic tree as styled lines, for clients whose
//! medium is lines (a terminal, a pipe, a log). A pixel or browser client does not
//! use this; it maps the tree to its own scene.
pub mod components;
pub mod lines;
pub mod select;
pub use lines::{Line, render, render_block, to_plain};
