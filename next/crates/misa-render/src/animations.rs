//! Named animation frames: the previous system's `misa.ui.animations` registry.
//!
//! An animation is data — a list of frames and an optional still — selected by a
//! role and advanced by the client's clock. Nothing here knows about time: a
//! frontend asks for the frame at a tick, exactly as the old `animations-frame`
//! did, and the clock lives with the surface that owns it.
//!
//! The registry is separate from colour on purpose. A frame is text; a theme
//! styles the span that carries it.

use std::collections::BTreeMap;

/// A frame sequence and the single frame shown when animation is off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Animation {
    pub frames: Vec<String>,
    pub still: Option<String>,
}

impl Animation {
    pub fn new(frames: &[&str], still: Option<&str>) -> Animation {
        Animation {
            frames: frames.iter().map(|frame| (*frame).to_string()).collect(),
            still: still.map(str::to_string),
        }
    }

    /// Whether this animation has more than one distinct frame.
    pub fn is_moving(&self) -> bool {
        self.frames.iter().any(|frame| frame != &self.frames[0])
    }

    /// The frame at a tick. Disabled animation uses the still frame, or the
    /// first frame when none was declared.
    pub fn frame(&self, enabled: bool, tick: u64) -> &str {
        if self.frames.is_empty() {
            return "";
        }
        if !enabled {
            return self.still.as_deref().unwrap_or(&self.frames[0]);
        }
        let index = (tick as usize) % self.frames.len();
        &self.frames[index]
    }
}

/// The shipped animation data. This is the old `animations.default` table.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    animations: BTreeMap<String, Animation>,
}

impl Registry {
    pub fn empty() -> Registry {
        Registry::default()
    }

    /// The shipped registry: `pulse`, `spinner` and `static`, as the previous
    /// system named them, with `pulse` as the default.
    pub fn stock() -> Registry {
        let mut registry = Registry::empty();
        registry
            .register("pulse", Animation::new(&["·", "•", "●", "•"], Some("…")))
            .expect("unique shipped animation");
        registry
            .register("spinner", Animation::new(&["·", "•", "●", "•"], None))
            .expect("unique shipped animation");
        registry
            .register("static", Animation::new(&["…"], None))
            .expect("unique shipped animation");
        registry
    }

    pub fn register(&mut self, id: &str, animation: Animation) -> Result<(), String> {
        if id.is_empty() || self.animations.contains_key(id) {
            return Err(format!("Duplicate or empty animation `{id}`"));
        }
        self.animations.insert(id.into(), animation);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&Animation> {
        self.animations.get(id)
    }

    /// The frame of a named animation, or `None` when no such animation exists.
    pub fn frame(&self, id: &str, enabled: bool, tick: u64) -> Option<&str> {
        self.get(id).map(|animation| animation.frame(enabled, tick))
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.animations.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_advances_with_the_tick_and_wraps() {
        let registry = Registry::stock();
        let frames: Vec<&str> = (0..5)
            .map(|tick| registry.frame("pulse", true, tick).unwrap())
            .collect();
        assert_eq!(frames, ["·", "•", "●", "•", "·"]);
    }

    #[test]
    fn a_disabled_animation_uses_its_still_frame() {
        let registry = Registry::stock();
        assert_eq!(registry.frame("pulse", false, 3), Some("…"));
        // A sequence with no still keeps its first frame.
        assert_eq!(registry.frame("spinner", false, 3), Some("·"));
    }

    #[test]
    fn a_static_sequence_never_moves() {
        let registry = Registry::stock();
        assert_eq!(registry.frame("static", true, 9), Some("…"));
        assert!(!registry.get("static").unwrap().is_moving());
        assert!(registry.get("pulse").unwrap().is_moving());
    }

    #[test]
    fn an_unknown_animation_has_no_frame() {
        assert_eq!(Registry::stock().frame("nope", true, 0), None);
    }

    #[test]
    fn registration_refuses_a_duplicate_or_empty_id() {
        let mut registry = Registry::empty();
        registry
            .register("pulse", Animation::new(&["a"], None))
            .unwrap();
        assert!(
            registry
                .register("pulse", Animation::new(&["b"], None))
                .is_err()
        );
        assert!(registry.register("", Animation::new(&["b"], None)).is_err());
    }
}
