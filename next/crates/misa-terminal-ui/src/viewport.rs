//! Semantic viewport position, independent of the document that supplies row keys.
//! Callbacks are only consulted when a reader scrolls, a selection moves, or the
//! layout changes; an unchanged repaint does not visit document rows.

/// A key and its ordinal within the consecutive run of rows carrying that key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor<K> {
    pub key: Option<K>,
    pub offset: usize,
}

/// The selected position: identity detects movement, row determines visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head<H> {
    pub identity: H,
    pub row: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Request<H> {
    pub scroll: usize,
    pub follow: bool,
    pub intent: u64,
    pub room: usize,
    pub head: Option<Head<H>>,
}

/// Holds viewport state across frames. `key_at` returns the key at a physical
/// row; `anchored_row` finds the current row for a key and run offset.
pub struct Viewport<K, H> {
    anchor: Option<Anchor<K>>,
    anchor_epoch: u64,
    anchor_row: usize,
    last_intent: u64,
    revealed_head: Option<H>,
    layout_epoch: u64,
    resolved_scroll: usize,
    following: bool,
}

impl<K: Eq, H: Copy + Eq> Viewport<K, H> {
    pub fn new(follow: bool) -> Self {
        Self {
            anchor: None,
            anchor_epoch: 0,
            anchor_row: 0,
            last_intent: 0,
            revealed_head: None,
            layout_epoch: 0,
            resolved_scroll: 0,
            following: follow,
        }
    }

    pub fn layout_changed(&mut self) {
        self.layout_epoch = self.layout_epoch.wrapping_add(1);
    }

    pub fn resolved_scroll(&self) -> usize {
        self.resolved_scroll
    }

    pub fn following(&self) -> bool {
        self.following
    }

    fn anchor_at(&self, row: usize, key_at: &mut impl FnMut(usize) -> Option<K>) -> Anchor<K> {
        let key = key_at(row);
        let mut offset = 0;
        if key.is_some() {
            let mut previous = row;
            while previous > 0 {
                previous -= 1;
                if key_at(previous).as_ref() != key.as_ref() {
                    break;
                }
                offset += 1;
            }
        }
        Anchor { key, offset }
    }

    /// Resolve the first row. A reader's physical scroll is anchored only after
    /// an intent; on layout changes the key restores the reader's position.
    pub fn resolve(
        &mut self,
        request: Request<H>,
        total: usize,
        mut key_at: impl FnMut(usize) -> Option<K>,
        mut anchored_row: impl FnMut(&K, usize) -> Option<usize>,
    ) -> usize {
        let Request {
            scroll,
            follow,
            intent,
            room,
            head,
        } = request;
        let bottom = total.saturating_sub(room);
        if follow {
            self.following = true;
        }
        let user_scrolled = self.last_intent != intent;
        if user_scrolled {
            self.following = scroll >= bottom;
        }
        if self.following {
            self.anchor = None;
            self.last_intent = intent;
            self.anchor_row = bottom;
            self.anchor_epoch = self.layout_epoch;
            self.resolved_scroll = bottom;
            return bottom;
        }
        let requested = scroll.min(bottom);
        if user_scrolled || self.anchor.is_none() {
            self.last_intent = intent;
            self.anchor = Some(self.anchor_at(requested, &mut key_at));
            self.anchor_row = requested;
            self.anchor_epoch = self.layout_epoch;
        } else if self.anchor_epoch != self.layout_epoch {
            self.anchor_row = self
                .anchor
                .as_ref()
                .and_then(|anchor| {
                    anchor
                        .key
                        .as_ref()
                        .and_then(|key| anchored_row(key, anchor.offset))
                })
                .unwrap_or(requested);
            self.anchor_epoch = self.layout_epoch;
        }
        if let Some(head) = head {
            if self.revealed_head != Some(head.identity) {
                self.revealed_head = Some(head.identity);
                if head.row < self.anchor_row {
                    self.anchor_row = head.row;
                } else if head.row >= self.anchor_row.saturating_add(room) {
                    self.anchor_row = head.row.saturating_add(1).saturating_sub(room);
                }
                self.anchor = Some(self.anchor_at(self.anchor_row, &mut key_at));
                self.anchor_epoch = self.layout_epoch;
            }
        } else {
            self.revealed_head = None;
        }
        self.resolved_scroll = self.anchor_row.min(bottom);
        self.resolved_scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(scroll: usize, intent: u64) -> Request<(usize, usize)> {
        Request {
            scroll,
            follow: false,
            intent,
            room: 3,
            head: None,
        }
    }

    #[test]
    fn run_offset_survives_growth_and_repaint_does_not_visit_keys() {
        let mut viewport = Viewport::<String, (usize, usize)>::new(false);
        let mut keys = vec!["a", "b", "b", "b", "c", "d", "e"];
        let start = viewport.resolve(
            request(2, 1),
            keys.len(),
            |row| keys.get(row).map(|s| s.to_string()),
            |_, _| panic!("new intent must not resolve old anchor"),
        );
        assert_eq!(start, 2);
        assert_eq!(
            viewport.anchor.as_ref(),
            Some(&Anchor {
                key: Some("b".to_string()),
                offset: 1
            })
        );
        assert_eq!(viewport.resolved_scroll(), 2);
        viewport.resolve(
            request(2, 1),
            keys.len(),
            |_| panic!("repaint scanned keys"),
            |_, _| panic!("repaint resolved key"),
        );
        keys.insert(0, "new");
        viewport.layout_changed();
        assert_eq!(
            viewport.resolve(
                request(2, 1),
                keys.len(),
                |_| panic!("layout should reuse anchor"),
                |key, offset| keys.iter().position(|k| *k == key).map(|row| row + offset)
            ),
            3
        );
    }

    #[test]
    fn removed_key_falls_back_to_requested_scroll() {
        let mut viewport = Viewport::<String, usize>::new(false);
        let request = Request {
            scroll: 2,
            follow: false,
            intent: 1,
            room: 3,
            head: None,
        };
        assert_eq!(
            viewport.resolve(request, 9, |row| Some(row.to_string()), |_, _| panic!()),
            2
        );
        viewport.layout_changed();
        assert_eq!(
            viewport.resolve(request, 8, |_| panic!("old key is cached"), |_, _| None),
            2
        );
    }

    #[test]
    fn follow_reveal_and_writeback() {
        let mut viewport = Viewport::<String, (usize, usize)>::new(true);
        let mut req = request(0, 0);
        req.follow = true;
        assert_eq!(viewport.resolve(req, 12, |_| panic!(), |_, _| panic!()), 9);
        req.follow = false;
        req.scroll = 1;
        req.intent = 1;
        assert_eq!(
            viewport.resolve(req, 12, |row| Some(row.to_string()), |_, _| panic!()),
            1
        );
        req.head = Some(Head {
            identity: (8, 0),
            row: 8,
        });
        assert_eq!(
            viewport.resolve(req, 12, |row| Some(row.to_string()), |_, _| panic!()),
            6
        );
        req.scroll = 2;
        req.intent = 2;
        assert_eq!(
            viewport.resolve(req, 12, |row| Some(row.to_string()), |_, _| panic!()),
            2
        );
        assert_eq!(viewport.resolved_scroll(), 2);
        req.scroll = 100;
        req.intent = 3;
        assert_eq!(viewport.resolve(req, 12, |_| panic!(), |_, _| panic!()), 9);
        assert!(viewport.following());
        assert_eq!(viewport.resolve(req, 15, |_| panic!(), |_, _| panic!()), 12);
    }
}
