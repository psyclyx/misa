//! Physical-pixel vertical viewport. Content coordinates are independent of painting.

/// Owns the scroll position and whether new content should remain at the tail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    offset: f32,
    follow: bool,
    content_height: f32,
    viewport_height: f32,
}

impl Viewport {
    pub fn new(content_height: f32, viewport_height: f32) -> Self {
        Self {
            offset: 0.0,
            follow: true,
            content_height,
            viewport_height,
        }
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn content_height(&self) -> f32 {
        self.content_height
    }

    pub fn viewport_height(&self) -> f32 {
        self.viewport_height
    }

    pub fn following(&self) -> bool {
        self.follow
    }

    /// A manual wheel move stops following, even if it is clamped at the edge.
    /// Wheel scrolling uses the content extent, not the caller's tail padding.
    pub fn scroll(&mut self, delta: f32) {
        self.offset =
            (self.offset + delta).clamp(0.0, (self.content_height - self.viewport_height).max(0.0));
        self.follow = false;
    }

    pub fn pin_to_top(&mut self) {
        self.follow = false;
        self.offset = 0.0;
    }

    pub fn follow_tail(&mut self) {
        self.follow = true;
    }

    /// Keep the current position when an interaction takes over from following.
    pub fn stop_following(&mut self) {
        self.follow = false;
    }

    /// Reconcile measured content and available height after layout. Returns whether
    /// the position changed and a layout using the previous position must be redone.
    /// Tail padding is an explicit caller policy, not a generic viewport constant.
    pub fn reconcile(
        &mut self,
        content_height: f32,
        viewport_height: f32,
        tail_padding: f32,
    ) -> bool {
        debug_assert!(content_height.is_finite() && content_height >= 0.0);
        debug_assert!(viewport_height.is_finite() && viewport_height >= 0.0);
        debug_assert!(tail_padding.is_finite() && tail_padding >= 0.0);
        self.content_height = content_height;
        self.viewport_height = viewport_height;
        let max = (content_height - viewport_height + tail_padding).max(0.0);
        let wanted = if self.follow {
            max
        } else {
            self.offset.min(max)
        };
        let changed = wanted != self.offset;
        self.offset = wanted;
        changed
    }

    /// Convert a content-space coordinate to viewport space.
    pub fn position(&self, content_y: f32) -> f32 {
        content_y - self.offset
    }

    /// Whether a half-open content-space interval intersects the viewport.
    pub fn visible(&self, top: f32, bottom: f32) -> bool {
        bottom > self.offset && top < self.offset + self.viewport_height
    }
}

#[cfg(test)]
mod tests {
    use super::Viewport;

    #[test]
    fn follows_tail_until_manually_scrolled_and_clamps_on_resize() {
        let mut view = Viewport::new(0.0, 600.0);
        assert!(view.following());
        assert!(!view.reconcile(100.0, 600.0, 40.0));
        assert!(view.reconcile(1000.0, 600.0, 40.0));
        assert_eq!(view.offset(), 440.0);
        assert!(view.visible(440.0, 441.0));
        assert!(!view.visible(0.0, 440.0));
        assert_eq!(view.position(450.0), 10.0);
        view.scroll(-50.0);
        assert_eq!(view.offset(), 390.0);
        assert!(!view.following());
        assert!(!view.reconcile(1200.0, 600.0, 40.0));
        view.scroll(10000.0);
        assert_eq!(view.offset(), 600.0); // wheel excludes tail padding
        assert!(view.reconcile(200.0, 600.0, 40.0));
        assert_eq!(view.offset(), 0.0);
        view.follow_tail();
        assert!(view.reconcile(1000.0, 600.0, 0.0));
        assert_eq!(view.offset(), 400.0);
        view.pin_to_top();
        assert!(!view.reconcile(1200.0, 600.0, 40.0));
        assert_eq!(view.offset(), 0.0);
    }
}
