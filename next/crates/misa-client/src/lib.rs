//! The client kit: what every frontend shares and no session owns.
//!
//! Three things live here, and the boundary between them and a session is the point
//! of the crate:
//!
//! - [`picker`] — choosing from a set of candidates. Matching, ranking, frecency,
//!   and the order of the list are here, because they are cheap, local, and
//!   different on every platform. *Which* candidates exist is not: that is a
//!   session's declaration and its data.
//! - [`editor`] — composing an input line. Modes, motions, history, and undo are
//!   here. What a submission *means* is not: that is an intent.
//! - [`intent`] — turning what a person did into the small set of things a client
//!   may ask for.
//!
//! Nothing here knows about a theme, a viewport, or a transport. A frontend draws
//! what this decides; a session answers what this asks.

pub mod editor;
pub mod intent;
pub mod picker;

pub use editor::{Editor, Mode, Motion};
pub use picker::{Accept, Accepted, Effect as PickerEffect, Frecency, Picker};
