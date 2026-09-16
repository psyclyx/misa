
//! Presentation for the linear frontends: text measurement, a role-addressed
//! theme, and a renderer from the semantic tree to styled lines.
//!
//! This crate sits on the client's side of the boundary and is the *only* place
//! the three shipped frontends share presentation code. It exists because two of
//! them are linear — a terminal and a plain pipe — and both need the same
//! wrapping, the same theme lookup, and the same decision about what a table
//! looks like when it is 80 columns wide.
//!
//! A pixel frontend does not use [`lines`]. It uses [`text`] for metrics and
//! [`theme`] for colour, and maps the same tree to a scene of its own. That split
//! is why this crate is not called a renderer: what it owns is presentation
//! *policy* for a medium, and the frontends own the painting.
//!
//! Nothing here knows about sessions, the wire, or the agent. It is a function of
//! `(tree, theme, width)`.

pub mod fact;
pub mod components;
pub mod lines;
pub mod text;
pub mod theme;

pub use lines::{Line, render, to_plain};
pub use text::{clip, pad, width, wrap_spans};
pub use theme::{Color, Style, Theme};
