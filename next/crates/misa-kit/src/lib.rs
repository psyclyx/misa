//! Surface-independent editor, picker, and local input preparation.

pub mod editor;
pub mod intent;
pub mod picker;

pub use editor::{Editor, Mode, Motion};
pub use picker::{Accept, Accepted, Effect as PickerEffect, Frecency, Picker};
