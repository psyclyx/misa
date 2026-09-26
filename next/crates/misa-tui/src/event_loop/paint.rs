//! Connected terminal presentation: image discovery, frame assembly and output retention.
use super::scopes::{Change, Scopes};
use crate::ConnectedScreen;
use misa_proto::view::BlobRef;
use std::{collections::HashSet, io::Write};

pub(super) struct ConnectedPainter {
    output: misa_terminal_ui::output::Output,
    animations: misa_render::animations::Registry,
    frame: usize,
    requested: HashSet<String>,
    images_dirty: bool,
    redraw: bool,
}

impl ConnectedPainter {
    pub fn new() -> Self {
        Self {
            output: Default::default(),
            animations: misa_render::animations::Registry::stock(),
            frame: 0,
            requested: HashSet::new(),
            images_dirty: true,
            redraw: true,
        }
    }

    pub fn changed(&mut self, change: Change) {
        self.images_dirty |= change.images();
        self.redraw = change.redraw();
    }

    pub fn redraw(&mut self) {
        self.redraw = true;
    }

    pub fn resized(&mut self) {
        self.output.invalidate();
        self.redraw();
    }

    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.redraw();
    }

    /// Called before selecting the next input. Discovery is update-driven, not paint-driven;
    /// a hash is requested once across all scope switches, even before its bytes arrive.
    pub fn downloads(&mut self, scopes: &Scopes, screen: &ConnectedScreen) -> Vec<BlobRef> {
        if !self.images_dirty {
            return vec![];
        }
        self.images_dirty = false;
        if !screen.ui.graphics.enabled() {
            return vec![];
        }
        scopes
            .image_blobs()
            .into_iter()
            .filter(|blob| {
                !screen.ui.graphics.has(&blob.hash) && self.requested.insert(blob.hash.clone())
            })
            .collect()
    }

    pub fn paint(
        &mut self,
        scopes: &mut Scopes,
        screen: &mut ConnectedScreen,
        writer: &mut impl Write,
    ) -> Result<(), String> {
        if !self.redraw {
            return Ok(());
        }
        let staging = (!scopes.active.pending.is_empty() || scopes.active.uploads > 0).then(|| {
            match (scopes.active.uploads > 0, scopes.active.pending.is_empty()) {
                (true, true) => "Loading image…".to_string(),
                (true, false) => "Loading image…\nRemove last attachment".to_string(),
                (false, false) => "Remove last attachment".to_string(),
                (false, true) => unreachable!("staging exists only for uploads or attachments"),
            }
        });
        let mut extra_document = vec![];
        let mut extra_footer = vec![];
        for contribution in scopes.active.contributions.values_mut() {
            contribution.local(&screen.ui);
            let (document, footer) = contribution.placed_lines(screen.ui.height as usize);
            extra_document.extend(document);
            extra_footer.extend(footer);
        }
        screen.refresh_dialog_surface();
        let mut rendered = scopes.active.retained.frame_with(
            &screen.ui,
            staging.as_deref(),
            &extra_document,
            &extra_footer,
        );
        // Resolve the semantic anchor to the displayed physical row before the next input.
        screen.ui.viewport_resolved(
            scopes.active.retained.resolved_scroll(),
            scopes.active.retained.following(),
        );
        if scopes.active.retained.has_turn() {
            if let Some(line) = rendered
                .lines
                .iter_mut()
                .find(|line| line.node.as_deref() == Some("indicators"))
            {
                let selected = misa_lines::components::animation_for(
                    screen.ui.component_settings(),
                    "activity",
                )
                .as_deref()
                .and_then(|id| self.animations.frame(id, true, self.frame as u64));
                if let Some(selected) = selected
                    && let Some((_, text)) = line.spans.iter_mut().find(|(_, text)| text == "●")
                {
                    *text = selected.to_string();
                }
            }
        }
        crate::terminal_loop::paint(writer, &mut self.output, &screen.ui, rendered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retained::Retained;
    use misa_proto::{Node, view::Kind};
    use misa_terminal_ui::graphics::{CellSize, Kitty};

    fn image(hash: &str) -> Node {
        Node::new(
            "picture",
            Kind::Image {
                blob: BlobRef {
                    hash: hash.into(),
                    len: 4,
                    media: Some("image/png".into()),
                },
                alt: "picture".into(),
                width: 1,
                height: 1,
            },
        )
        .id("picture")
    }

    #[test]
    fn unchanged_paint_retains_rows_and_images_are_requested_only_on_document_changes() {
        let mut screen = ConnectedScreen::new(80, 24);
        screen.ui.graphics = Kitty::new(true, CellSize::default());
        let mut scopes = Scopes::new(&screen);
        scopes.active.retained =
            Retained::new(Node::section("session").child(image("a")), &screen.ui);
        let mut painter = ConnectedPainter::new();
        assert_eq!(painter.downloads(&scopes, &screen)[0].hash, "a");
        screen.ui.graphics.insert("a", image::RgbaImage::new(1, 1));
        scopes.active.retained.local(&screen.ui);
        let mut bytes = Vec::new();
        painter.paint(&mut scopes, &mut screen, &mut bytes).unwrap();
        assert!(
            String::from_utf8_lossy(&bytes).contains("a=T"),
            "kitty placement missing"
        );
        bytes.clear();
        painter.paint(&mut scopes, &mut screen, &mut bytes).unwrap();
        assert!(
            !bytes.windows(4).any(|part| part == b"\x1b[2K"),
            "unchanged rows were rewritten"
        );
        assert!(
            !String::from_utf8_lossy(&bytes).contains("a=T"),
            "unchanged kitty placement was retransmitted"
        );
        assert!(painter.downloads(&scopes, &screen).is_empty());

        scopes.active.retained =
            Retained::new(Node::section("session").child(image("b")), &screen.ui);
        assert!(
            painter.downloads(&scopes, &screen).is_empty(),
            "paint must not discover images"
        );
        painter.changed(Change::Documents);
        assert_eq!(painter.downloads(&scopes, &screen)[0].hash, "b");
        painter.paint(&mut scopes, &mut screen, &mut bytes).unwrap();
        painter.changed(Change::Documents);
        assert!(
            painter.downloads(&scopes, &screen).is_empty(),
            "already requested hashes must stay requested"
        );
    }

    #[test]
    fn redraw_and_animation_phase_are_gated_independently_of_image_discovery() {
        let mut screen = ConnectedScreen::new(80, 24);
        let mut scopes = Scopes::new(&screen);
        scopes.active.retained = Retained::new(
            Node::section("session")
                .child(Node::section("turn").id("turn"))
                .child(
                    Node::section("status.indicators").id("indicators").child(
                        Node::new(
                            "indicator.activity",
                            Kind::Status {
                                text: String::new(),
                            },
                        )
                        .id("activity"),
                    ),
                ),
            &screen.ui,
        );
        assert!(scopes.active.retained.has_turn());
        let mut painter = ConnectedPainter::new();
        let mut bytes = Vec::new();
        painter.paint(&mut scopes, &mut screen, &mut bytes).unwrap();
        assert!(
            String::from_utf8_lossy(&bytes).contains('·'),
            "initial activity frame missing"
        );
        painter.changed(Change::None);
        bytes.clear();
        painter.paint(&mut scopes, &mut screen, &mut bytes).unwrap();
        assert!(bytes.is_empty(), "no-change update must skip paint");
        assert_eq!(painter.frame, 0);
        painter.tick();
        assert_eq!(painter.frame, 1);
        painter.paint(&mut scopes, &mut screen, &mut bytes).unwrap();
        assert!(
            String::from_utf8_lossy(&bytes).contains('•'),
            "animation tick must change the activity frame"
        );
        assert!(painter.downloads(&scopes, &screen).is_empty());
    }
}
