//! Semantic session updates and their paint invalidation.
use super::{
    Update, enqueue,
    scopes::{Change, Scopes},
};
use crate::{ConnectedScreen, SessionReply, SessionRequest as Request};
use tokio::sync::mpsc;

pub(super) fn handle_update(
    update: Option<Update>,
    scopes: &mut Scopes,
    screen: &mut ConnectedScreen,
    commands: &mpsc::UnboundedSender<Request>,
) -> Result<Option<Change>, String> {
    let mut change = Change::Redraw;
    match update {
        Some(Update::View(crate::Presentation::Forget(id))) => {
            change = scopes.forget(&id, screen);
        }
        Some(Update::View(crate::Presentation::Documents(documents))) => {
            for (id, update) in documents {
                if id.is_empty() {
                    crate::document_adapter::observed(
                        &mut scopes.active.retained,
                        &update,
                        &screen.ui,
                    )?;
                } else if let misa_client::document::Update::Unavailable(fault) = &update {
                    scopes.active.contributions.remove(&id);
                    screen.ui.notice = Some(format!("{id}: {}", fault.message));
                } else {
                    let contribution = scopes.active.contributions.entry(id).or_insert_with(|| {
                        crate::retained::Retained::new(
                            misa_proto::Node::section("presentation").id("presentation"),
                            &screen.ui,
                        )
                    });
                    crate::document_adapter::observed(contribution, &update, &screen.ui)?;
                }
            }
            change = Change::Documents;
        }
        Some(Update::View(crate::Presentation::Activate(next))) => {
            change = scopes.activate(next, screen);
        }
        Some(Update::View(crate::Presentation::Contribution { id, update })) => match &update {
            misa_client::document::Update::Unavailable(fault) => {
                scopes.active.contributions.remove(&id);
                screen.ui.notice = Some(format!("{id}: {}", fault.message));
            }
            _ => {
                let contribution = scopes.active.contributions.entry(id).or_insert_with(|| {
                    crate::retained::Retained::new(
                        misa_proto::Node::section("presentation").id("presentation"),
                        &screen.ui,
                    )
                });
                crate::document_adapter::observed(contribution, &update, &screen.ui)?;
                change = Change::Documents;
            }
        },
        Some(Update::View(crate::Presentation::Attention { id, generation })) => {
            screen.dialogs.focus_request(id, generation);
            enqueue(&commands, Request::RefreshRequests, screen);
        }
        Some(Update::View(crate::Presentation::Reply(SessionReply::Request {
            id,
            generation,
            model,
        }))) => screen.dialogs.update(id, generation, model),
        Some(Update::View(crate::Presentation::Reply(SessionReply::Report(report)))) => {
            screen.dialogs.report(report)
        }
        Some(Update::View(crate::Presentation::Reply(SessionReply::DaemonForm {
            daemon,
            scope,
            form,
            drafts,
        }))) => screen.dialogs.daemon_form(daemon, scope, form, drafts),
        Some(Update::View(crate::Presentation::Reply(SessionReply::Form(form)))) => {
            screen.dialogs.form(form)
        }
        Some(Update::View(crate::Presentation::TurnOutput(_))) => {}
        Some(Update::View(crate::Presentation::Declaration { catalog, location })) => {
            screen.declare(&catalog);
            screen.ui.location = location;
        }
        Some(Update::View(crate::Presentation::Candidates {
            source,
            items,
            truncated,
        })) => screen.ui.candidates(&source, items, truncated),
        Some(Update::View(crate::Presentation::Snapshot(next))) => {
            scopes.active.retained = crate::retained::Retained::new(next, &screen.ui);
            change = Change::Documents;
        }
        Some(Update::View(crate::Presentation::Document(update))) => {
            match &update {
                misa_client::document::Update::Unavailable(fault) => {
                    screen.ui.notice = Some(fault.message.clone())
                }
                // Status is a semantic indicator rendered by the retained
                // status owner. Turning it into a debug notice duplicates the
                // status bar and makes normal activity look like an error.
                misa_client::document::Update::Status(_) => {}
                _ => {}
            }
            crate::document_adapter::observed(&mut scopes.active.retained, &update, &screen.ui)?;
            change = Change::Documents;
        }
        Some(Update::View(crate::Presentation::Reply(SessionReply::Complete {
            source,
            prefix,
            result,
        }))) => match result {
            Ok((items, truncated)) => {
                screen.ui.completion(&source, &prefix, items, truncated);
            }
            Err(error) => {
                screen.ui.completion_failed(&source);
                screen.ui.notice = Some(error);
            }
        },
        Some(Update::View(crate::Presentation::Reply(SessionReply::Notice(notice)))) => {
            screen.ui.notice = Some(notice)
        }
        Some(Update::View(crate::Presentation::Reply(SessionReply::Uploaded {
            generation: reply_generation,
            result,
        }))) => {
            if let Some(outcome) = scopes.uploaded(reply_generation, result) {
                screen.ui.notice = Some(match outcome {
                    Ok(()) => "Image attached; Enter sends the prompt".into(),
                    Err(error) => error,
                });
            }
        }
        Some(Update::View(crate::Presentation::Reply(SessionReply::Downloaded {
            reference,
            result,
        }))) => match result {
            Ok(bytes) => match image::load_from_memory(&bytes) {
                Ok(image) => screen
                    .ui
                    .graphics
                    .insert(&reference.hash, image.into_rgba8()),
                Err(error) => screen.ui.notice = Some(format!("Image decode failed: {error}")),
            },
            Err(error) => screen.ui.notice = Some(error),
        },
        Some(Update::View(crate::Presentation::Reply(SessionReply::Sent { draft, result }))) => {
            match result {
                Ok(()) => screen.ui.notice = None,
                Err(error) => {
                    if let Some((text, attachments)) = draft {
                        screen.ui.composer.prepend(&text);
                        scopes.active.pending.extend(attachments);
                    }
                    screen.ui.notice = Some(error);
                }
            }
        }
        None => return Ok(None),
    }
    Ok(Some(change))
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        Node,
        view::{BlobRef, Kind},
    };
    use misa_terminal_ui::graphics::{CellSize, Kitty};

    #[test]
    fn document_image_is_downloaded_once_through_update_and_cache() {
        let mut screen = ConnectedScreen::new(80, 24);
        screen.ui.graphics = Kitty::new(true, CellSize::default());
        let mut scopes = Scopes::new(&screen);
        let mut painter = super::super::paint::ConnectedPainter::new();
        let (commands, mut requests) = mpsc::unbounded_channel();
        let reference = BlobRef {
            hash: "document-image".into(),
            len: 4,
            media: Some("image/png".into()),
        };
        let document = || {
            misa_client::document::Update::Reset(misa_proto::observation::Document {
                version: misa_proto::sync::Version {
                    epoch: "image".into(),
                    rev: 0,
                },
                tree: Node::section("session").id("session").child(
                    Node::new(
                        "picture",
                        Kind::Image {
                            blob: reference.clone(),
                            alt: "picture".into(),
                            width: 1,
                            height: 1,
                        },
                    )
                    .id("picture"),
                ),
                streams: vec![],
            })
        };
        assert!(painter.downloads(&scopes, &screen).is_empty());
        for _ in 0..2 {
            let change = handle_update(
                Some(Update::View(crate::Presentation::Document(document()))),
                &mut scopes,
                &mut screen,
                &commands,
            )
            .unwrap()
            .unwrap();
            assert_eq!(change, Change::Documents);
            painter.changed(change);
            for blob in painter.downloads(&scopes, &screen) {
                super::super::enqueue(
                    &commands,
                    Request::Download { reference: blob },
                    &mut screen,
                );
            }
        }
        assert!(
            matches!(requests.try_recv(), Ok(Request::Download { reference: blob }) if blob == reference)
        );
        assert!(
            requests.try_recv().is_err(),
            "duplicate download before reply"
        );

        let bytes = crate::clipboard::png(1, 1, vec![255, 0, 0, 255]).unwrap();
        let change = handle_update(
            Some(Update::View(crate::Presentation::Reply(
                SessionReply::Downloaded {
                    reference: reference.clone(),
                    result: Ok(bytes),
                },
            ))),
            &mut scopes,
            &mut screen,
            &commands,
        )
        .unwrap()
        .unwrap();
        painter.changed(change);
        assert!(screen.ui.graphics.has(&reference.hash));
        let change = handle_update(
            Some(Update::View(crate::Presentation::Document(document()))),
            &mut scopes,
            &mut screen,
            &commands,
        )
        .unwrap()
        .unwrap();
        painter.changed(change);
        assert!(painter.downloads(&scopes, &screen).is_empty());
        assert!(requests.try_recv().is_err(), "cached image requested again");
    }

    fn handle(
        presentation: crate::Presentation,
        scopes: &mut Scopes,
        screen: &mut ConnectedScreen,
    ) -> Change {
        let (commands, _requests) = mpsc::unbounded_channel();
        handle_update(Some(Update::View(presentation)), scopes, screen, &commands)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn document_changes_and_scope_transitions_have_typed_invalidation() {
        let mut screen = ConnectedScreen::new(80, 24);
        let mut scopes = Scopes::new(&screen);
        assert_eq!(
            handle(
                crate::Presentation::Snapshot(Node::section("first")),
                &mut scopes,
                &mut screen,
            ),
            Change::Documents
        );
        assert_eq!(
            handle(
                crate::Presentation::Activate("A".into()),
                &mut scopes,
                &mut screen,
            ),
            Change::Redraw
        );
        assert_eq!(
            handle(
                crate::Presentation::Activate("A".into()),
                &mut scopes,
                &mut screen,
            ),
            Change::None
        );
        assert_eq!(
            handle(
                crate::Presentation::Activate("B".into()),
                &mut scopes,
                &mut screen,
            ),
            Change::Documents
        );
        assert_eq!(
            handle(
                crate::Presentation::Forget("A".into()),
                &mut scopes,
                &mut screen,
            ),
            Change::None
        );
        let (commands, _requests) = mpsc::unbounded_channel();
        assert_eq!(
            handle_update(None, &mut scopes, &mut screen, &commands).unwrap(),
            None
        );
    }

    #[test]
    fn late_upload_reply_does_not_attach_to_the_visible_scope() {
        let mut screen = ConnectedScreen::new(80, 24);
        let mut scopes = Scopes::new(&screen);
        handle(
            crate::Presentation::Activate("A".into()),
            &mut scopes,
            &mut screen,
        );
        let old = scopes.active.generation;
        scopes.active.uploads = 1;
        handle(
            crate::Presentation::Activate("B".into()),
            &mut scopes,
            &mut screen,
        );
        let blob = BlobRef {
            hash: "from-A".into(),
            len: 1,
            media: None,
        };
        assert_eq!(
            handle(
                crate::Presentation::Reply(SessionReply::Uploaded {
                    generation: old,
                    result: Ok(blob),
                }),
                &mut scopes,
                &mut screen,
            ),
            Change::Redraw
        );
        assert!(scopes.active.pending.is_empty());
        assert_eq!(screen.ui.notice, None);
        handle(
            crate::Presentation::Activate("A".into()),
            &mut scopes,
            &mut screen,
        );
        assert_eq!(scopes.active.pending[0].hash, "from-A");
        assert_eq!(scopes.active.uploads, 0);
    }
}
