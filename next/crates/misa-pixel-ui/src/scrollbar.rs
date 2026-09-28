//! The one scrollbar: track, thumb, and the same fitting rules as every other
//! control. A container that cannot state its extent exactly draws no thumb —
//! one that lies about where the reader is costs more than none at all.
use crate::flow::Scroll;
use crate::{Op, Rect};
use misa_style::Style;
use std::sync::Arc;

/// The shortest thumb. Proportional size is the honest size; this floor only
/// keeps something grabbable when the ratio would be a sliver.
pub const MIN_THUMB: f32 = 4.0;

pub struct Scrollbar<Id> {
    pub id: Id,
    /// The track the thumb travels along.
    pub bounds: Rect,
    /// Exact container metrics; nothing here is estimated.
    pub scroll: Scroll,
    pub track: Style,
    pub thumb_style: Style,
}

pub struct PlacedScrollbar<Id> {
    pub id: Id,
    pub bounds: Rect,
    pub thumb: Rect,
    pub scroll: Scroll,
    pub ops: Vec<Op>,
}

impl<Id> PlacedScrollbar<Id> {
    pub fn hit(&self, x: f32, y: f32) -> bool {
        self.bounds.contains(x, y)
    }

    /// The content offset a thumb dragged with its center at `y` asks for.
    pub fn drag(&self, y: f32) -> f32 {
        let travel = (self.bounds.height - self.thumb.height).max(0.0);
        if travel <= 0.0 {
            return self.scroll.offset;
        }
        let fraction = ((y - self.thumb.height / 2.0 - self.bounds.y) / travel).clamp(0.0, 1.0);
        fraction * (self.scroll.content - self.scroll.window).max(0.0)
    }

    /// The content offset one page above or below the thumb, or `None` for a
    /// click on the thumb itself.
    pub fn page(&self, y: f32) -> Option<f32> {
        if self.thumb.contains(self.thumb.x, y) {
            return None;
        }
        if y < self.thumb.y {
            Some((self.scroll.offset - self.scroll.window).max(0.0))
        } else {
            Some(self.scroll.offset + self.scroll.window)
        }
    }
}

impl<Id> Scrollbar<Id> {
    pub fn place(self) -> PlacedScrollbar<Id> {
        let travel = self.bounds.height.max(0.0);
        let content = self.scroll.content.max(self.scroll.window);
        let thumb_height = if content > 0.0 {
            (travel * self.scroll.window / content).clamp(MIN_THUMB.min(travel), travel)
        } else {
            travel
        };
        let room = (travel - thumb_height).max(0.0);
        let span = (content - self.scroll.window).max(0.0);
        let y = if span > 0.0 {
            self.bounds.y + room * (self.scroll.offset / span).clamp(0.0, 1.0)
        } else {
            self.bounds.y
        };
        let thumb = Rect {
            x: self.bounds.x,
            y,
            width: self.bounds.width,
            height: thumb_height,
        };
        // Pill track and pill thumb: chrome, not another content box.
        let track = Rect {
            x: self.bounds.x + (self.bounds.width - 2.0).max(0.0) / 2.0,
            y: self.bounds.y,
            width: 2.0,
            height: self.bounds.height.max(0.0),
        };
        PlacedScrollbar {
            id: self.id,
            ops: vec![Op::ClipRect {
                x: self.bounds.x,
                y: self.bounds.y,
                width: self.bounds.width.max(0.0),
                height: self.bounds.height.max(0.0),
                ops: Arc::new(vec![
                    Op::RoundedRect {
                        x: track.x,
                        y: track.y,
                        width: track.width,
                        height: track.height,
                        radius: 1.0,
                        style: self.track,
                    },
                    Op::RoundedRect {
                        x: thumb.x,
                        y: thumb.y,
                        width: thumb.width.max(0.0),
                        height: thumb.height,
                        radius: self.bounds.width / 2.0,
                        style: self.thumb_style,
                    },
                ]),
            }],
            bounds: self.bounds,
            thumb,
            scroll: self.scroll,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(offset: f32, content: f32, window: f32) -> PlacedScrollbar<u8> {
        Scrollbar {
            id: 7,
            bounds: Rect {
                x: 390.0,
                y: 0.0,
                width: 6.0,
                height: 100.0,
            },
            scroll: Scroll {
                content,
                window,
                offset,
            },
            track: Style::default(),
            thumb_style: Style::default(),
        }
        .place()
    }

    #[test]
    fn thumb_size_and_position_reflect_exact_metrics() {
        let placed = bar(450.0, 1000.0, 100.0);
        // Proportional size is the honest size: 10% of the track.
        assert_eq!(placed.thumb.height, 10.0);
        // 450 of the 900 scrollable pixels: halfway down the travel.
        assert_eq!(placed.thumb.y, 45.0);
    }

    #[test]
    fn tiny_content_gets_a_draggable_thumb() {
        let placed = bar(10.0, 5_000.0, 100.0);
        assert_eq!(placed.thumb.height, MIN_THUMB);
        assert!(placed.thumb.y >= 0.0 && placed.thumb.y + MIN_THUMB <= 100.0);
        // A thumb larger than its proportional share still reads the ratio.
        let roomy = bar(0.0, 200.0, 100.0);
        assert_eq!(roomy.thumb.height, 50.0);
    }

    #[test]
    fn unscrolled_content_holds_the_thumb_at_the_top() {
        let placed = bar(0.0, 40.0, 100.0);
        assert_eq!(placed.thumb.height, 100.0);
        assert_eq!(placed.thumb.y, 0.0);
    }

    #[test]
    fn dragging_and_paging_map_to_content_offsets() {
        let placed = bar(0.0, 1000.0, 100.0);
        // Dragging the thumb's center to the bottom asks for the last page.
        assert_eq!(placed.drag(100.0), 900.0);
        assert_eq!(placed.drag(50.0), 450.0);
        assert_eq!(placed.page(8.0), None, "the thumb itself does not page");
        assert_eq!(placed.page(80.0), Some(100.0));
        let reading = bar(200.0, 1000.0, 100.0);
        assert_eq!(reading.page(5.0), Some(100.0));
    }
}
