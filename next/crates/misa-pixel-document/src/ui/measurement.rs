//! Immutable, owner-scoped inputs for a future height worker. No live UI state crosses
//! this boundary: even a secret editor is represented by its displayed bullets.
use super::{DocumentUi, flow::FlowId};
use misa_pixel_ui::TextMetrics;
use misa_proto::view::{FieldKind, Kind, Node};
use misa_render::Theme;
use std::collections::BTreeMap;
use std::sync::Arc;

#[cfg(test)]
#[path = "measurement_tests.rs"]
mod tests;

/// A single atomic owner's production layout inputs, independent of the live UI.
/// The width is the *content* width passed to `measure_owner`, not the window width.
pub(super) struct HeightSnapshot {
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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SnapshotError {
    NotAtomic,
    MissingOwner,
}

impl HeightSnapshot {
    pub(super) fn capture(ui: &DocumentUi, id: &FlowId, width: f32) -> Result<Self, SnapshotError> {
        let theme = if ui.light {
            Theme::light()
        } else {
            Theme::dark()
        };
        let mut row_content_index = None;
        let mut columns = None;
        let (owner, mut root, include_streams) = match id {
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
                        Kind::List {
                            ordered: *ordered,
                            items: vec![
                                items
                                    .get(*index)
                                    .ok_or(SnapshotError::MissingOwner)?
                                    .clone(),
                            ],
                            markers: vec![markers.get(*index).copied().flatten()],
                        }
                    }
                    Kind::Table { head, rows, align } if *index == 0 => Kind::Table {
                        head: head.clone(),
                        rows: vec![],
                        align: align.clone(),
                    },
                    Kind::Table { rows, align, .. } => {
                        row_content_index = Some(1);
                        Kind::Table {
                            head: vec![],
                            rows: vec![
                                rows.get(index - 1)
                                    .ok_or(SnapshotError::MissingOwner)?
                                    .clone(),
                            ],
                            align: align.clone(),
                        }
                    }
                    _ => return Err(SnapshotError::MissingOwner),
                };
                (id.clone(), Node::new(&original.role, kind).id(owner), false)
            }
            FlowId::End(owner) if ui.document.row_count(owner).is_some() => {
                let original = ui.document.node(owner).ok_or(SnapshotError::MissingOwner)?;
                let mut root = Node::new(&original.role, empty_flow_kind(&original.kind)).id(owner);
                root.children = ui
                    .document
                    .children(owner)
                    .iter()
                    .filter_map(|child| ui.document.subtree(child))
                    .collect();
                root.actions = original.actions.clone();
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
                    let mut root =
                        Node::new(&original.role, empty_flow_kind(&original.kind)).id(id);
                    root.label = original.label.clone();
                    (FlowId::Node(id.clone()), root, false)
                } else {
                    let root = ui.document.subtree(id).ok_or(SnapshotError::MissingOwner)?;
                    let include_streams = contains_id(&root, ui.document.stream_parent());
                    (FlowId::Node(id.clone()), root, include_streams)
                }
            }
            FlowId::Stream(id) if ui.document.has_stream(id) => (
                FlowId::Stream(id.clone()),
                ui.document
                    .stream_or_node(id)
                    .ok_or(SnapshotError::MissingOwner)?
                    .clone(),
                false,
            ),
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
        if include_streams {
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

fn contains_id(node: &Node, id: &str) -> bool {
    node.id == id || node.children.iter().any(|child| contains_id(child, id))
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
                let value = ui.drafts.text(&node.id, &field.id).unwrap_or(&field.value);
                field.value = if field.secret {
                    "•".repeat(value.chars().count())
                } else {
                    value.to_owned()
                };
                if let FieldKind::Choice { options, selected } = &mut field.kind {
                    if field.secret {
                        // Neither choices nor their selected value are needed for a masked control.
                        options.clear();
                        *selected = None;
                    } else if !field.read_only {
                        // Drafts::reset initializes choices from `selected`, not `value`.
                        *selected = Some(field.value.clone());
                    }
                }
            }
        }
        Kind::Image { blob, .. } => {
            if let Some(image) = ui.document.image_ref(&blob.hash) {
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

/// Run the actual production owner layout in a private UI. Its display list and
/// geometry are dropped here; neither a scene nor GPU resources leave this call.
pub(super) fn measure(snapshot: HeightSnapshot, metrics: Arc<dyn TextMetrics>) -> f32 {
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
    match &snapshot.owner {
        FlowId::Node(id) if id == "streams" => builder.measure_owner(id, snapshot.width, &theme),
        _ => builder.measure_flow(&snapshot.owner, snapshot.width, &theme),
    }
    .expect("captured atomic owner must exist in private document")
}
