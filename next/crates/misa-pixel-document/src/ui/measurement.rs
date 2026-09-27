//! Immutable, owner-scoped inputs for the background layout worker. No live UI
//! state crosses this boundary: even a secret editor is represented by its
//! displayed bullets, never by the text behind them.
use super::{DocumentUi, flow::FlowId, retained::RetainedScenes};
use misa_pixel_ui::TextMetrics;
use misa_proto::view::{FieldKind, Kind, Node};
use misa_render::Theme;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;

#[cfg(test)]
#[path = "measurement_tests.rs"]
mod tests;

/// One owner's serialized layout inputs must fit this budget before any of them
/// is cloned. An atomic ancestor cannot enqueue an unbounded stream projection.
pub(super) const SNAPSHOT_BUDGET: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SnapshotError {
    NotAtomic,
    MissingOwner,
    /// The owner alone exceeds [`SNAPSHOT_BUDGET`]; only the frame may measure it.
    Oversize,
    /// The poll's shared capture budget is spent; retry on a later poll.
    Budget,
}

/// A single atomic owner's production layout inputs, independent of the live UI.
/// The width is the *content* width passed to `measure_owner`, not the window width.
pub(super) struct OwnerSnapshot {
    owner: FlowId,
    row_content_index: Option<usize>,
    columns: Option<usize>,
    root: Node,
    streams: Vec<Node>,
    images: BTreeMap<String, Arc<image::RgbaImage>>,
    expanded: Vec<String>,
    width: f32,
    light: bool,
}

impl OwnerSnapshot {
    pub(super) fn capture(
        ui: &DocumentUi,
        id: &FlowId,
        width: f32,
        budget: &mut usize,
    ) -> Result<Self, SnapshotError> {
        // Every arm counts its owner-scoped inputs before cloning any of them,
        // so an atomic ancestor cannot copy an unbounded subtree or projection.
        let mut cost = 0usize;
        let theme = if ui.light {
            Theme::light()
        } else {
            Theme::dark()
        };
        let mut row_content_index = None;
        let mut columns = None;
        let mut only_stream = None;
        let (owner, mut root, streams_planned) = match id {
            FlowId::Row(owner, index) if ui.document.row_count(owner).is_some() => {
                let original = ui.document.node(owner).ok_or(SnapshotError::MissingOwner)?;
                columns = matches!(original.kind, Kind::Table { .. })
                    .then(|| ui.document.table_columns(owner));
                let kind = match &original.kind {
                    Kind::List {
                        ordered,
                        items,
                        markers,
                    } => {
                        row_content_index = Some(0);
                        let item = items.get(*index).ok_or(SnapshotError::MissingOwner)?;
                        count(&mut cost, *budget, item)?;
                        charge(
                            &mut cost,
                            *budget,
                            item.iter()
                                .map(|node| inline_draft_cost(node, ui))
                                .sum::<usize>(),
                        )?;
                        Kind::List {
                            ordered: *ordered,
                            items: vec![item.clone()],
                            markers: vec![markers.get(*index).copied().flatten()],
                        }
                    }
                    Kind::Table { head, rows, align } if *index == 0 => {
                        count(&mut cost, *budget, head)?;
                        count(&mut cost, *budget, align)?;
                        Kind::Table {
                            head: head.clone(),
                            rows: vec![],
                            align: align.clone(),
                        }
                    }
                    Kind::Table { rows, align, .. } => {
                        row_content_index = Some(1);
                        let row = rows.get(index - 1).ok_or(SnapshotError::MissingOwner)?;
                        count(&mut cost, *budget, row)?;
                        count(&mut cost, *budget, align)?;
                        Kind::Table {
                            head: vec![],
                            rows: vec![row.clone()],
                            align: align.clone(),
                        }
                    }
                    _ => return Err(SnapshotError::MissingOwner),
                };
                (id.clone(), shell(original, owner, kind), false)
            }
            FlowId::End(owner) if ui.document.row_count(owner).is_some() => {
                let original = ui.document.node(owner).ok_or(SnapshotError::MissingOwner)?;
                for child in ui.document.children(owner) {
                    subtree_cost(&ui.document, ui, &child, &mut cost, *budget)?;
                }
                count(&mut cost, *budget, &original.actions)?;
                let mut root = shell(original, owner, empty_flow_kind(&original.kind));
                root.children = ui
                    .document
                    .children(owner)
                    .iter()
                    .filter_map(|child| ui.document.subtree(child))
                    .collect();
                let include_streams = owner == ui.document.stream_parent();
                (id.clone(), root, include_streams)
            }
            // `retain_owner` resolves this synthetic group before the tree lookup.
            FlowId::Node(id) if id == "streams" => (
                FlowId::Node(id.clone()),
                Node::section("measurement").id("__measurement_root"),
                true,
            ),
            FlowId::Node(id) if ui.document.contains(id) && !ui.document.structural(id, &theme) => {
                let original = ui.document.node(id).ok_or(SnapshotError::MissingOwner)?;
                if ui.document.row_count(id).is_some() {
                    let root = shell(original, id, empty_flow_kind(&original.kind));
                    (FlowId::Node(id.clone()), root, false)
                } else {
                    subtree_cost(&ui.document, ui, id, &mut cost, *budget)?;
                    let root = ui.document.subtree(id).ok_or(SnapshotError::MissingOwner)?;
                    let include_streams = contains_id(&root, ui.document.stream_parent());
                    (FlowId::Node(id.clone()), root, include_streams)
                }
            }
            FlowId::Stream(id) if ui.document.has_stream(id) => {
                let node = ui
                    .document
                    .stream_or_node(id)
                    .ok_or(SnapshotError::MissingOwner)?;
                count(&mut cost, *budget, node)?;
                charge(&mut cost, *budget, inline_draft_cost(node, ui))?;
                // A live projection must not become a tree node in the private
                // store: the painter finds it in the stream index and renders
                // its children inline, exactly as it does live.
                only_stream = Some(id.clone());
                (
                    FlowId::Stream(id.clone()),
                    Node::section("measurement").id("__measurement_root"),
                    false,
                )
            }
            FlowId::Node(_) | FlowId::Stream(_) | FlowId::Row(_, _) | FlowId::End(_) => {
                return Err(SnapshotError::MissingOwner);
            }
            FlowId::Top | FlowId::Close(_) | FlowId::StreamClose | FlowId::Bottom => {
                return Err(SnapshotError::NotAtomic);
            }
        };
        let mut images = BTreeMap::new();
        let mut expanded = Vec::new();
        prepare(&mut root, ui, &mut images, &mut expanded);
        let mut streams = Vec::new();
        if let Some(id) = &only_stream {
            let mut stream = ui
                .document
                .stream_or_node(id)
                .ok_or(SnapshotError::MissingOwner)?
                .clone();
            prepare(&mut stream, ui, &mut images, &mut expanded);
            streams.push(stream);
        }
        if streams_planned {
            for id in ui.document.visible_streams() {
                // Count every projected stream before cloning any of them.
                let node = ui
                    .document
                    .stream_or_node(&id)
                    .ok_or(SnapshotError::MissingOwner)?;
                count(&mut cost, *budget, node)?;
                charge(&mut cost, *budget, inline_draft_cost(node, ui))?;
            }
            for id in ui.document.visible_streams() {
                let mut stream = ui
                    .document
                    .stream_or_node(&id)
                    .ok_or(SnapshotError::MissingOwner)?
                    .clone();
                prepare(&mut stream, ui, &mut images, &mut expanded);
                streams.push(stream);
            }
        }
        *budget -= cost;
        Ok(Self {
            owner,
            row_content_index,
            columns,
            root,
            streams,
            images,
            expanded,
            width,
            light: ui.light,
        })
    }
}

fn empty_flow_kind(kind: &Kind) -> Kind {
    match kind {
        Kind::List { ordered, .. } => Kind::List {
            ordered: *ordered,
            items: vec![],
            markers: vec![],
        },
        Kind::Table { align, .. } => Kind::Table {
            head: vec![],
            rows: vec![],
            align: align.clone(),
        },
        _ => unreachable!(),
    }
}

/// An owner's own presentation fields travel with every shrunk shell: the rail
/// and surface decisions in `retain_flow` read them from the owner node.
fn shell(original: &Node, id: &str, kind: Kind) -> Node {
    let mut node = Node::new(&original.role, kind).id(id);
    node.label = original.label.clone();
    node.state = original.state;
    node.actions = original.actions.clone();
    node
}

fn contains_id(node: &Node, id: &str) -> bool {
    node.id == id || node.children.iter().any(|child| contains_id(child, id))
}

/// Add one planned input's serialized size to the running total. The owner cap
/// reports [`SnapshotError::Oversize`]; the poll's shared budget reports
/// [`SnapshotError::Budget`]. Either way it fails before the caller clones.
fn count<T: Serialize + ?Sized>(
    cost: &mut usize,
    budget: usize,
    value: &T,
) -> Result<(), SnapshotError> {
    *cost += serialized_len(value, SNAPSHOT_BUDGET.saturating_sub(*cost))?;
    charge(cost, budget, 0)
}

/// Charge unserialized input (draft text) against the same budgets.
fn charge(cost: &mut usize, budget: usize, extra: usize) -> Result<(), SnapshotError> {
    *cost += extra;
    if *cost > budget {
        return Err(SnapshotError::Budget);
    }
    Ok(())
}

/// Count what `DocumentStore::subtree(id)` will clone: every indexed
/// descendant's shell, its inline payloads, and the drafts that replace values.
fn subtree_cost(
    document: &super::document::DocumentStore,
    ui: &DocumentUi,
    id: &str,
    cost: &mut usize,
    budget: usize,
) -> Result<(), SnapshotError> {
    let node = document.node(id).ok_or(SnapshotError::MissingOwner)?;
    count(cost, budget, node)?;
    charge(cost, budget, inline_draft_cost(node, ui))?;
    for child in document.children(id) {
        subtree_cost(document, ui, &child, cost, budget)?;
    }
    Ok(())
}

/// Serialized input size, counted without allocating the serialization. The cap
/// aborts the count early: an oversize owner costs at most one budget's walk.
fn serialized_len<T: Serialize + ?Sized>(value: &T, cap: usize) -> Result<usize, SnapshotError> {
    struct Count(usize, usize);
    impl Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            if self.0 > self.1 {
                Err(std::io::Error::other("snapshot budget exceeded"))
            } else {
                Ok(buf.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Count(0, cap);
    serde_json::to_writer(&mut counter, value).map_err(|_| SnapshotError::Oversize)?;
    Ok(counter.0)
}

/// Editor drafts replace field values in the snapshot, so their text is part of
/// the input size even when the tree's own value is empty. Covers one node's
/// inline payloads: indexed descendants are walked by `subtree_cost`.
fn inline_draft_cost(node: &Node, ui: &DocumentUi) -> usize {
    let mut cost = 0usize;
    if let Kind::Fields { fields } = &node.kind {
        for field in fields {
            cost += ui
                .drafts
                .text(&node.id, &field.id)
                .map(str::len)
                .unwrap_or(0);
        }
    }
    cost + node
        .children
        .iter()
        .map(|child| inline_draft_cost(child, ui))
        .sum::<usize>()
        + match &node.kind {
            Kind::List { items, .. } => items
                .iter()
                .flatten()
                .map(|child| inline_draft_cost(child, ui))
                .sum(),
            _ => 0,
        }
}

fn prepare(
    node: &mut Node,
    ui: &DocumentUi,
    images: &mut BTreeMap<String, Arc<image::RgbaImage>>,
    expanded: &mut Vec<String>,
) {
    if let Kind::Collapsible { .. } = node.kind
        && ui.interaction.is_expanded(&node.id)
    {
        expanded.push(node.id.clone());
    }
    match &mut node.kind {
        Kind::Fields { fields } => {
            for field in fields {
                // Mirror the painter's displayed value exactly: a Choice without
                // a draft shows its selected value, everything else `field.value`.
                let default = match &field.kind {
                    FieldKind::Choice {
                        selected: Some(value),
                        ..
                    } => value.as_str(),
                    _ => field.value.as_str(),
                };
                let value = ui.drafts.text(&node.id, &field.id).unwrap_or(default);
                field.value = if field.secret {
                    "•".repeat(value.chars().count())
                } else {
                    value.to_owned()
                };
                if let FieldKind::Choice { options, selected } = &mut field.kind {
                    if field.secret {
                        // A masked control's height depends only on its bullets,
                        // and no option label may leak into the worker.
                        options.clear();
                        *selected = None;
                    } else {
                        // The painter resolves choices against the displayed
                        // value; keep `selected` on that same value.
                        *selected = Some(field.value.clone());
                    }
                }
            }
        }
        Kind::Image { blob, .. } => {
            if let Some(image) = ui.document.image_ref(&blob.hash) {
                // Decoded pixels stay owned by the live document's bounded cache;
                // the snapshot only carries the shared handle.
                images.insert(blob.hash.clone(), Arc::clone(image));
            }
        }
        Kind::List { items, .. } => {
            for child in items.iter_mut().flatten() {
                prepare(child, ui, images, expanded);
            }
        }
        _ => {}
    }
    for child in &mut node.children {
        prepare(child, ui, images, expanded);
    }
}

pub(super) struct Rendered {
    pub(super) height: f32,
    pub(super) retained: RetainedScenes,
}

/// Run the actual production owner layout in a private UI and keep its display
/// list. No GPU resource crosses this call; the group is plain paint input.
pub(super) fn render(snapshot: OwnerSnapshot, metrics: Arc<dyn TextMetrics>) -> Rendered {
    let mut ui = DocumentUi::new(snapshot.root, metrics);
    if let Some(columns) = snapshot.columns {
        if let FlowId::Row(owner, _) = &snapshot.owner {
            ui.document.set_measurement_columns(owner, columns);
        }
    }
    ui.set_light(snapshot.light);
    ui.document
        .install_measurement_resources(snapshot.streams, snapshot.images);
    for id in snapshot.expanded {
        ui.interaction.toggle_disclosure(&id);
    }
    let theme = if ui.light {
        Theme::light()
    } else {
        Theme::dark()
    };
    let mut builder = super::layout::LayoutBuilder::new(
        &ui.document,
        &mut ui.drafts,
        &mut ui.interaction,
        &mut ui.retained,
        &mut ui.overlays,
        ui.metrics.as_ref(),
        ui.light,
    );
    builder.row_content_index = snapshot.row_content_index;
    let height = match &snapshot.owner {
        FlowId::Node(id) if id == "streams" => builder.measure_owner(id, snapshot.width, &theme),
        _ => builder.measure_flow(&snapshot.owner, snapshot.width, &theme),
    }
    .expect("captured atomic owner must exist in private document");
    Rendered {
        height,
        retained: ui.retained,
    }
}

/// Exact height alone, for callers that keep no display list.
#[cfg(test)]
pub(super) fn measure(snapshot: OwnerSnapshot, metrics: Arc<dyn TextMetrics>) -> f32 {
    render(snapshot, metrics).height
}
