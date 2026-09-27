//! Indexed document order for the viewport. Undecorated sections contribute only
//! their trailing 5px spacing; painted components remain indivisible owners.
use super::document::DocumentStore;
use super::layout::LayoutBuilder;
use misa_pixel_ui::{FlowPlacement, FlowPosition, FlowSource, FlowViewport};
use misa_proto::view::Kind;
use misa_render::Theme;
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum FlowId {
    Top,
    Node(String),
    /// Positional row identity: the wire format has no stable item/row IDs.
    Row(String, usize),
    End(String),
    Close(String),
    Stream(String),
    StreamClose,
    Bottom,
}

impl DocumentStore {
    pub(super) fn structural(&self, id: &str, theme: &Theme) -> bool {
        self.node(id).is_some_and(|node| {
            matches!(node.kind, Kind::Section)
                && node.label.is_none()
                && node.actions.is_empty()
                && theme.rail(&node.role).is_none()
                && theme.surface(&node.role).is_none()
                && !matches!(
                    node.role.as_str(),
                    "status.indicators" | "message.group.footer" | "queue"
                )
        })
    }
    pub(super) fn row_count(&self, id: &str) -> Option<usize> {
        let node = self.node(id)?;
        if matches!(
            node.role.as_str(),
            "status.indicators" | "message.group.footer" | "queue"
        ) {
            return None;
        }
        match &node.kind {
            Kind::List { items, .. } => Some(items.len()),
            Kind::Table { rows, .. } => Some(rows.len() + 1), // header is row zero
            _ => None,
        }
    }
    fn first_flow(&self, id: &str, theme: &Theme) -> FlowId {
        if self.structural(id, theme) {
            if let Some(child) = self.tree.first_child(id) {
                return self.first_flow(child, theme);
            }
            if id == self.stream_parent() {
                if let Some(stream) = self.first_stream() {
                    return FlowId::Stream(stream);
                }
            }
            FlowId::Close(id.to_owned())
        } else {
            FlowId::Node(id.to_owned())
        }
    }
    fn last_flow(&self, id: &str, theme: &Theme) -> FlowId {
        if self.structural(id, theme) {
            FlowId::Close(id.to_owned())
        } else if self.row_count(id).is_some() {
            FlowId::End(id.to_owned())
        } else {
            FlowId::Node(id.to_owned())
        }
    }
    fn after_flow(&self, id: &str, theme: &Theme) -> FlowId {
        if let Some(sibling) = self.tree.next_sibling(id) {
            return self.first_flow(sibling, theme);
        }
        let Some(parent) = self.parent(id) else {
            return FlowId::Bottom;
        };
        if parent == self.stream_parent() {
            if let Some(stream) = self.first_stream() {
                return FlowId::Stream(stream);
            }
        }
        FlowId::Close(parent.to_owned())
    }
    fn before_flow(&self, id: &str, theme: &Theme) -> FlowId {
        if let Some(sibling) = self.tree.previous_sibling(id) {
            return self.last_flow(sibling, theme);
        }
        let Some(parent) = self.parent(id) else {
            return FlowId::Top;
        };
        if self.structural(parent, theme) {
            self.before_flow(parent, theme)
        } else {
            FlowId::Node(parent.to_owned())
        }
    }
    pub(super) fn previous_flow(&self, id: &FlowId, theme: &Theme) -> Option<FlowId> {
        match id {
            FlowId::Top => None,
            FlowId::Bottom => Some(self.last_flow(self.root(), theme)),
            FlowId::Node(id) => Some(self.before_flow(id, theme)),
            FlowId::Row(id, index) => Some(if *index == 0 {
                FlowId::Node(id.clone())
            } else {
                FlowId::Row(id.clone(), index - 1)
            }),
            FlowId::End(id) => Some(self.row_count(id).filter(|count| *count > 0).map_or_else(
                || FlowId::Node(id.clone()),
                |count| FlowId::Row(id.clone(), count - 1),
            )),
            FlowId::Close(id) => {
                if id == self.stream_parent() && self.first_stream().is_some() {
                    return Some(FlowId::StreamClose);
                }
                Some(self.tree.last_child(id).map_or_else(
                    || self.before_flow(id, theme),
                    |child| self.last_flow(child, theme),
                ))
            }
            FlowId::StreamClose => self.last_stream().map(FlowId::Stream),
            FlowId::Stream(id) => Some(if let Some(previous) = self.previous_stream(id) {
                FlowId::Stream(previous)
            } else if let Some(child) = self.tree.last_child(self.stream_parent()) {
                self.last_flow(child, theme)
            } else {
                self.before_flow(self.stream_parent(), theme)
            }),
        }
    }
    pub(super) fn next_flow(&self, id: &FlowId, theme: &Theme) -> Option<FlowId> {
        match id {
            FlowId::Top => Some(self.first_flow(self.root(), theme)),
            FlowId::Bottom => None,
            FlowId::StreamClose => Some(FlowId::Close(self.stream_parent().to_owned())),
            FlowId::Node(id) => Some(self.row_count(id).map_or_else(
                || self.after_flow(id, theme),
                |count| {
                    if count == 0 {
                        FlowId::End(id.clone())
                    } else {
                        FlowId::Row(id.clone(), 0)
                    }
                },
            )),
            FlowId::Row(id, index) => Some(if index + 1 < self.row_count(id)? {
                FlowId::Row(id.clone(), index + 1)
            } else {
                FlowId::End(id.clone())
            }),
            FlowId::End(id) | FlowId::Close(id) => Some(self.after_flow(id, theme)),
            FlowId::Stream(id) => Some(
                self.next_stream(id)
                    .map_or_else(|| FlowId::StreamClose, FlowId::Stream),
            ),
        }
    }
    // Reset-only walks: ordinals count painted reading owners, not margins or
    // structural closing spacers. Neither walk measures heights or runs on a frame.
    fn reading_ordinal(&self, target: &FlowId, theme: &Theme) -> Option<usize> {
        let mut cursor = FlowId::Top;
        let mut count = 0usize;
        loop {
            if &cursor == target {
                return Some(if cursor.reading_owner() {
                    count
                } else {
                    count.saturating_sub(1)
                });
            }
            if cursor.reading_owner() {
                count += 1;
            }
            cursor = self.next_flow(&cursor, theme)?;
        }
    }
    fn reset_reading_at(
        &self,
        ordinal: usize,
        wanted: &HashSet<FlowId>,
        theme: &Theme,
    ) -> (FlowId, HashSet<FlowId>) {
        let mut cursor = FlowId::Top;
        let mut last = FlowId::Top;
        let mut at_ordinal = None;
        let mut surviving = HashSet::new();
        let mut count = 0;
        while let Some(next) = self.next_flow(&cursor, theme) {
            if next.reading_owner() {
                if wanted.contains(&next) {
                    surviving.insert(next.clone());
                }
                if count == ordinal {
                    at_ordinal = Some(next.clone());
                }
                last = next.clone();
                count += 1;
            }
            cursor = next;
        }
        (at_ordinal.unwrap_or(last), surviving)
    }
    pub(super) fn contains_flow(&self, id: &FlowId, theme: &Theme) -> bool {
        match id {
            FlowId::Top | FlowId::Bottom => true,
            FlowId::Node(id) => self.contains(id) && !self.structural(id, theme),
            FlowId::Row(id, index) => {
                self.row_count(id).is_some_and(|count| *index < count)
                    && !self.structural(id, theme)
            }
            FlowId::End(id) => self.row_count(id).is_some() && !self.structural(id, theme),
            FlowId::Close(id) => self.contains(id) && self.structural(id, theme),
            FlowId::Stream(id) => self.has_stream(id),
            FlowId::StreamClose => self.first_stream().is_some(),
        }
    }
}

impl FlowId {
    pub(super) fn cache_key(&self) -> String {
        match self {
            Self::Node(id) | Self::Stream(id) => id.clone(),
            Self::Row(id, index) => format!("\0row:{id}:{index}"),
            Self::End(id) => format!("\0end:{id}"),
            _ => unreachable!("spacers have no display list"),
        }
    }
    fn reading_owner(&self) -> bool {
        matches!(self, Self::Node(_) | Self::Row(_, _) | Self::Stream(_))
    }
}

/// Captured before a reset, restored only after the replacement has been indexed.
/// Visible placements provide an exact screen point for surviving owners; the
/// ordinal is used only when none survive (never an estimate of pixel height).
pub(super) struct ResetAnchor {
    id: FlowId,
    ordinal: usize,
    local_y: f32,
    screen_y: f32,
    visible: Vec<FlowPlacement<FlowId>>,
}
impl ResetAnchor {
    pub(super) fn capture(
        document: &DocumentStore,
        viewport: &FlowViewport<FlowId>,
        theme: &Theme,
    ) -> Option<Self> {
        let FlowPosition::Anchor {
            id,
            local_y,
            screen_y,
        } = &viewport.position
        else {
            return None; // New documents and live tail followers still follow the tail.
        };
        Some(Self {
            ordinal: document.reading_ordinal(id, theme)?,
            id: id.clone(),
            local_y: *local_y,
            screen_y: *screen_y,
            visible: viewport.visible().to_vec(),
        })
    }

    pub(super) fn restore(
        self,
        document: &DocumentStore,
        viewport: &mut FlowViewport<FlowId>,
        theme: &Theme,
    ) {
        if !self.id.reading_owner() && document.contains_flow(&self.id, theme) {
            return;
        }
        // Check actual reading-order membership, not just tree containment: a
        // surviving node may now be inside an atomic painted parent.
        let wanted: HashSet<_> = self
            .visible
            .iter()
            .map(|p| p.id.clone())
            .chain(std::iter::once(self.id.clone()))
            .collect();
        let (ordinal_owner, surviving) = document.reset_reading_at(self.ordinal, &wanted, theme);
        if surviving.contains(&self.id) {
            return;
        }
        if let Some(placement) = self
            .visible
            .iter()
            .filter(|p| surviving.contains(&p.id))
            .min_by(|a, b| {
                (a.y - self.screen_y)
                    .abs()
                    .total_cmp(&(b.y - self.screen_y).abs())
            })
        {
            viewport.anchor(placement.id.clone(), 0.0, placement.y);
        } else {
            viewport.anchor(ordinal_owner, self.local_y, self.screen_y);
        }
    }
}

impl FlowSource for LayoutBuilder<'_> {
    type Id = FlowId;
    fn first(&self) -> Option<FlowId> {
        Some(FlowId::Top)
    }
    fn last(&self) -> Option<FlowId> {
        Some(FlowId::Bottom)
    }
    fn previous(&self, id: &FlowId) -> Option<FlowId> {
        self.document.previous_flow(id, &self.theme)
    }
    fn next(&self, id: &FlowId) -> Option<FlowId> {
        self.document.next_flow(id, &self.theme)
    }
    fn contains(&self, id: &FlowId) -> bool {
        self.document.contains_flow(id, &self.theme)
    }
    fn measure(&mut self, id: &FlowId, width: f32, _: u64) -> f32 {
        match id {
            FlowId::Top => 20.0,
            FlowId::Bottom => 60.0, // bottom margin and transcript breathing room
            FlowId::Close(_) | FlowId::StreamClose => 5.0,
            FlowId::Node(_) | FlowId::Row(_, _) | FlowId::End(_) | FlowId::Stream(_) => {
                let theme = self.theme.clone();
                self.measure_flow(id, width, &theme)
                    .expect("indexed flow owner")
            }
        }
    }
}
