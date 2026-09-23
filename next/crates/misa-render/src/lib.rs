//! Shared presentation primitives for clients.
//!
//! Medium-agnostic: text measurement, a role-addressed theme, typed-value
//! formatting, and animation frame data. Every shipped frontend may use these,
//! and none of them decides what a medium looks like.
//!
//! The *linear* renderer — the semantic tree as styled lines — is `misa-lines`,
//! used by the terminal and pipe clients. A pixel or browser client uses [`text`]
//! for metrics and [`theme`] for colour and maps the tree to a scene of its own; a
//! client that wants syntax colouring uses `misa-syntax` on that side too.
//!
//! Nothing here knows about sessions, the wire, or the agent. It is a function of
//! `(tree, theme, width)`.

pub mod animations;
pub mod fact;
pub mod local_offset;
pub mod text;
pub mod theme;

pub use animations::{Animation, Registry as Animations};
pub use text::{clip, columns, pad, width, wrap_spans, wrap_styled};
pub use theme::{Color, Palette, Style, StylePatch, Theme, ThemeOverrides, alert_role};
