//! Terminal-local event routing and host intent dispatch.
use super::{enqueue, paint::ConnectedPainter, scopes::Scopes};
use crate::{ConnectedScreen, KeyOut, SessionRequest as Request};
use crossterm::event::{self, Event, KeyEventKind};
use std::io::Write;
use tokio::sync::mpsc;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum InputOutcome {
    Continue,
    Quit,
}

pub(super) fn offered_actions(
    view: &misa_proto::Node,
    contributions: &std::collections::BTreeMap<String, crate::retained::Retained>,
) -> std::collections::BTreeMap<String, (String, String, String)> {
    fn visit(
        node: &misa_proto::Node,
        origin: &str,
        actions: &mut std::collections::BTreeMap<String, (String, String, String)>,
    ) {
        for action in &node.actions {
            actions.insert(
                action.id.clone(),
                (
                    node.id.clone(),
                    action.label.clone().unwrap_or_else(|| action.id.clone()),
                    origin.into(),
                ),
            );
        }
        for child in &node.children {
            visit(child, origin, actions);
        }
        if let misa_proto::view::Kind::List { items, .. } = &node.kind {
            for item in items {
                for child in item {
                    visit(child, origin, actions);
                }
            }
        }
    }
    let mut actions = std::collections::BTreeMap::new();
    visit(view, "Conversation", &mut actions);
    for (id, document) in contributions {
        visit(&document.interaction(), id, &mut actions);
    }
    actions
}
// Route terminal input without borrowing the session or the request owner.
pub(super) fn handle_input(
    event: Event,
    scopes: &mut Scopes,
    screen: &mut ConnectedScreen,
    commands: &mpsc::UnboundedSender<Request>,
    writer: &mut impl Write,
    clipboard: &mut dyn crate::clipboard::Source,
    painter: &mut ConnectedPainter,
) -> Result<InputOutcome, String> {
    painter.redraw();
    let view = scopes.active.retained.interaction();
    let out = match event {
        Event::Resize(width, height) => {
            screen.ui.width = width;
            screen.ui.height = height;
            scopes.active.retained.local(&screen.ui);
            painter.resized();
            return Ok(InputOutcome::Continue);
        }
        Event::Paste(text) => {
            if screen.dialogs.paste(&text) {
            } else if crate::panel_of(&view).is_some() {
                for character in text.chars() {
                    screen.ui.panel_key(&view, &crate::Key::Char(character));
                }
            } else {
                paste(screen, &text, &commands);
            }
            scopes.active.retained.local(&screen.ui);
            return Ok(InputOutcome::Continue);
        }
        Event::Mouse(mouse) => {
            // The wheel is transcript scroll, three rows per notch, through the
            // same handler the keyboard scroll actions use.
            match mouse.kind {
                event::MouseEventKind::ScrollUp => {
                    let _ = screen.ui.key(crate::Key::ScrollPage(-3));
                }
                event::MouseEventKind::ScrollDown => {
                    let _ = screen.ui.key(crate::Key::ScrollPage(3));
                }
                _ => return Ok(InputOutcome::Continue),
            }
            scopes.active.retained.local(&screen.ui);
            return Ok(InputOutcome::Continue);
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            if key.code == event::KeyCode::Char('v')
                && key.modifiers.contains(event::KeyModifiers::CONTROL)
            {
                if key.modifiers.contains(event::KeyModifiers::ALT) {
                    scopes.discard_attachments();
                    screen.ui.notice = Some("Clipboard attachments discarded".into());
                    return Ok(InputOutcome::Continue);
                }
                match clipboard.read() {
                    Ok(crate::clipboard::Contents::Text(text)) => {
                        if screen.dialogs.paste(&text) {
                        } else if crate::panel_of(&view).is_some() {
                            for character in text.chars() {
                                screen.ui.panel_key(&view, &crate::Key::Char(character));
                            }
                        } else {
                            paste(screen, &text, &commands);
                        }
                    }
                    Ok(crate::clipboard::Contents::Image {
                        width,
                        height,
                        rgba,
                    }) => {
                        if screen.dialogs.focused() || crate::panel_of(&view).is_some() {
                            screen.ui.notice = Some(
                                "Close the input form to attach an image to the prompt".into(),
                            );
                            return Ok(InputOutcome::Continue);
                        }
                        if scopes.active.pending.len() + scopes.active.uploads >= 16 {
                            screen.ui.notice = Some(
                                "Send or discard staged attachments before adding more".into(),
                            );
                            return Ok(InputOutcome::Continue);
                        }
                        match crate::clipboard::png(width, height, rgba) {
                            Ok(bytes) => {
                                if enqueue(
                                    &commands,
                                    Request::Upload {
                                        generation: scopes.active.generation,
                                        bytes,
                                        media: "image/png".into(),
                                    },
                                    screen,
                                ) {
                                    scopes.active.uploads += 1;
                                }
                            }
                            Err(error) => screen.ui.notice = Some(error),
                        }
                    }
                    Err(error) => screen.ui.notice = Some(error),
                }
                scopes.active.retained.local(&screen.ui);
                return Ok(InputOutcome::Continue);
            }
            if key.code == event::KeyCode::Enter
                && !screen.dialogs.focused()
                && !key.modifiers.contains(event::KeyModifiers::SHIFT)
                && scopes.active.uploads > 0
            {
                screen.ui.notice = Some("Wait for the attachment upload before sending".into());
                return Ok(InputOutcome::Continue);
            }
            let queued = view.children.iter().any(|node| node.id == "queue");
            if key.code == event::KeyCode::Enter
                && !screen.dialogs.focused()
                && screen.ui.composer.text().is_empty()
                && crate::panel_of(&view).is_none()
                && !key.modifiers.contains(event::KeyModifiers::SHIFT)
                && (queued || !scopes.active.pending.is_empty())
            {
                if key.modifiers.contains(event::KeyModifiers::ALT) && queued {
                    KeyOut::Intent(misa_kit::intent::Intent::Action {
                        node: "queue".into(),
                        action: "queue.steer".into(),
                        args: misa_value::Value::Null,
                        fields: vec![],
                    })
                } else if key.modifiers.contains(event::KeyModifiers::ALT) {
                    KeyOut::Intent(misa_kit::intent::Intent::Interrupt {
                        text: String::new(),
                        attachments: vec![],
                    })
                } else {
                    KeyOut::Intent(misa_kit::intent::Intent::Prompt {
                        text: String::new(),
                        attachments: vec![],
                    })
                }
            } else {
                let Some(key) = crate::translate(key.code, key.modifiers, screen.ui.keymap())
                else {
                    return Ok(InputOutcome::Continue);
                };
                // Ctrl-C has the reference's two meanings: while a turn is
                // active it asks the owner to cancel; while the composer is
                // otherwise idle it discards the local draft. The distinction
                // belongs to this input router because only it has both the
                // rendered session actions and the terminal-local draft.
                if key == crate::Key::Interrupt
                    && !screen.ui.composer.has_picker()
                    && !screen.dialogs.focused()
                    && !screen.ui.panel_active()
                    && !screen.ui.has_selection()
                    && !has_action(&view, "turn.cancel")
                {
                    screen.ui.composer.set_text("");
                    screen.ui.notice = None;
                    return Ok(InputOutcome::Continue);
                }
                if matches!(key, crate::Key::QueueEdit)
                    && !screen.dialogs.focused()
                    && crate::panel_of(&view).is_none()
                    && view.children.iter().any(|node| {
                        node.id == "queue"
                            && node.actions.iter().any(|action| action.id == "queue.edit")
                    })
                {
                    KeyOut::Intent(misa_kit::intent::Intent::Action {
                        node: "queue".into(),
                        action: "queue.edit".into(),
                        args: misa_value::Value::Null,
                        fields: vec![],
                    })
                } else {
                    let dialog_keys = screen.ui.dialog_settings().clone();
                    match screen.dialogs.key(&key, &dialog_keys) {
                        Some(crate::dialogs::DialogOut::Ui(out)) => out,
                        Some(crate::dialogs::DialogOut::DaemonInvoke {
                            daemon,
                            scope,
                            command,
                            input,
                        }) => {
                            enqueue(
                                &commands,
                                Request::DaemonInvoke {
                                    daemon,
                                    scope,
                                    command,
                                    input,
                                },
                                screen,
                            );
                            scopes.active.retained.local(&screen.ui);
                            return Ok(InputOutcome::Continue);
                        }
                        None => crate::terminal_loop::route_key(
                            &mut screen.ui,
                            &mut scopes.active.retained,
                            &view,
                            key,
                        ),
                    }
                }
            }
        }
        _ => return Ok(InputOutcome::Continue),
    };
    match out {
        KeyOut::Invoke { command, input } => {
            enqueue(&commands, Request::Invoke { command, input }, screen);
        }
        KeyOut::Local => {}
        KeyOut::Quit => return Ok(InputOutcome::Quit),
        KeyOut::Intent(mut intent) => {
            if matches!(&intent,misa_kit::intent::Intent::Command{name,..} if name=="actions") {
                let actions = offered_actions(&view, &scopes.active.contributions);
                let mut picker = crate::Picker::new("Document actions", crate::Accept::Run);
                picker.set_items(
                    actions
                        .iter()
                        .map(|(id, (_, label, origin))| misa_proto::view::Choice {
                            value: format!("/action {id}"),
                            label: label.clone(),
                            detail: Some(origin.clone()),
                            metadata: None,
                        })
                        .collect(),
                    false,
                );
                screen.ui.composer.open_host_picker(picker, ":");
                return Ok(InputOutcome::Continue);
            }
            if let misa_kit::intent::Intent::Command { name, args } = &intent
                && name == "action"
            {
                let actions = offered_actions(&view, &scopes.active.contributions);
                let id = args
                    .get("action")
                    .and_then(misa_value::Value::as_str)
                    .unwrap_or("");
                let Some((node, _, _)) = actions.get(id) else {
                    screen.ui.notice =
                        Some("Action is no longer offered by a visible document".into());
                    return Ok(InputOutcome::Continue);
                };
                intent = misa_kit::intent::Intent::Action {
                    node: node.clone(),
                    action: id.into(),
                    args: misa_value::Value::Null,
                    fields: vec![],
                };
            }
            if matches!(&intent, misa_kit::intent::Intent::Command { name, .. } if name == "operations")
            {
                screen.dialogs.open();
                enqueue(&commands, Request::RefreshRequests, screen);
                return Ok(InputOutcome::Continue);
            }
            let draft = match &mut intent {
                misa_kit::intent::Intent::Prompt { text, attachments }
                | misa_kit::intent::Intent::Interrupt { text, attachments } => {
                    attachments.extend(scopes.active.pending.iter().cloned());
                    Some(text.clone())
                }
                _ => None,
            };
            if enqueue(&commands, Request::Intent(intent), screen) {
                if draft.is_some() {
                    scopes.active.pending.clear();
                }
            } else if let Some(text) = draft {
                screen.ui.composer.set_text(text);
            }
        }
        KeyOut::Complete { source, prefix } => {
            enqueue(&commands, Request::Complete { source, prefix }, screen);
        }
        KeyOut::Submitted(text) => match crate::save::parse(&text) {
            Some(Ok(request)) => match crate::save::target(&view, &request) {
                Ok(node) => {
                    enqueue(
                        &commands,
                        Request::Save {
                            node: node.into(),
                            destination: request.destination,
                        },
                        screen,
                    );
                }
                Err(error) => screen.ui.notice = Some(error),
            },
            Some(Err(error)) => {
                screen.ui.composer.set_text(text);
                screen.ui.save();
                screen.ui.notice = Some(error);
            }
            None => screen.ui.notice = Some("Unknown host command".into()),
        },
        KeyOut::Copy(text) => {
            misa_terminal_ui::clipboard::write(writer, &text).map_err(|error| error.to_string())?;
        }
        KeyOut::StartSelection => {
            // Selection needs the retained document's rendered geometry;
            // keep that operation at the client/view boundary rather than
            // smuggling a second document copy into the UI screen.
            let _ = scopes
                .active
                .retained
                .selection_key(&mut screen.ui, &crate::Key::StartSelection);
        }
    }
    scopes.active.retained.local(&screen.ui);
    Ok(InputOutcome::Continue)
}

fn has_action(node: &misa_proto::Node, id: &str) -> bool {
    node.actions.iter().any(|action| action.id == id)
        || node.children.iter().any(|child| has_action(child, id))
        || match &node.kind {
            misa_proto::view::Kind::List { items, .. } => {
                items.iter().flatten().any(|child| has_action(child, id))
            }
            _ => false,
        }
}

fn paste(screen: &mut ConnectedScreen, text: &str, commands: &mpsc::UnboundedSender<Request>) {
    if let KeyOut::Complete { source, prefix } = screen.ui.paste(text) {
        enqueue(commands, Request::Complete { source, prefix }, screen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ImageClipboard;
    impl crate::clipboard::Source for ImageClipboard {
        fn read(&mut self) -> Result<crate::clipboard::Contents, String> {
            Ok(crate::clipboard::Contents::Image {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            })
        }
    }

    fn key(code: event::KeyCode, modifiers: event::KeyModifiers) -> Event {
        Event::Key(event::KeyEvent::new(code, modifiers))
    }

    #[test]
    fn image_upload_carries_generation_and_discard_rejects_late_reply() {
        let mut screen = ConnectedScreen::new(80, 24);
        let mut scopes = Scopes::new(&screen);
        let (commands, mut requests) = mpsc::unbounded_channel();
        let mut painter = ConnectedPainter::new();
        let mut writer = Vec::new();
        let mut clipboard = ImageClipboard;
        let ctrl = event::KeyModifiers::CONTROL;
        let mut input = |event| {
            handle_input(
                event,
                &mut scopes,
                &mut screen,
                &commands,
                &mut writer,
                &mut clipboard,
                &mut painter,
            )
            .unwrap()
        };
        assert_eq!(
            input(key(event::KeyCode::Char('v'), ctrl)),
            InputOutcome::Continue
        );
        let Request::Upload {
            generation,
            bytes,
            media,
        } = requests.try_recv().unwrap()
        else {
            panic!("clipboard image did not queue an upload");
        };
        assert_eq!(media, "image/png");
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 1);
        assert_eq!(
            input(key(event::KeyCode::Enter, event::KeyModifiers::NONE)),
            InputOutcome::Continue
        );
        assert!(requests.try_recv().is_err(), "Enter must wait for upload");
        assert_eq!(
            input(key(
                event::KeyCode::Char('v'),
                ctrl | event::KeyModifiers::ALT
            )),
            InputOutcome::Continue
        );
        drop(input);
        assert_eq!(scopes.active.uploads, 0);
        assert_ne!(scopes.active.generation, generation);
        assert!(
            scopes
                .uploaded(
                    generation,
                    Ok(misa_proto::view::BlobRef {
                        hash: "late".into(),
                        len: 1,
                        media: None,
                    }),
                )
                .is_none()
        );
        assert!(scopes.active.pending.is_empty());
        assert_eq!(
            handle_input(
                key(event::KeyCode::Char('q'), ctrl),
                &mut scopes,
                &mut screen,
                &commands,
                &mut writer,
                &mut clipboard,
                &mut painter,
            )
            .unwrap(),
            InputOutcome::Quit
        );
    }

    #[test]
    fn focused_host_dialog_takes_enter_before_queue_and_upload_but_cannot_block_quit() {
        use misa_proto::{Node, view::BlobRef};
        let mut screen = ConnectedScreen::new(80, 24);
        let mut scopes = Scopes::new(&screen);
        scopes.active.retained = crate::retained::Retained::new(
            Node::section("session").id("session").child(
                Node::section("queue").id("queue").child(
                    Node::text("queued", [misa_proto::view::Span::plain("next prompt")])
                        .id("queued-item"),
                ),
            ),
            &screen.ui,
        );
        scopes.active.pending.push(BlobRef {
            hash: "staged".into(),
            len: 1,
            media: None,
        });
        scopes.active.uploads = 1;
        screen.dialogs.update(
            "approval".into(),
            1,
            Some(misa_client::request::Model {
                form: None,
                id: "approval".into(),
                generation: 1,
                title: "Approve".into(),
                body: Node::section("request"),
                input: None,
                actions: vec![misa_client::request::Action {
                    id: "resolve".into(),
                    label: "Approve".into(),
                    binding: misa_proto::invocation::ActionBinding {
                        command: "input.resolve".into(),
                        bound: Default::default(),
                        inputs: Default::default(),
                    },
                }],
            }),
        );
        screen.dialogs.open();
        assert!(screen.dialogs.focused());
        let (commands, mut requests) = mpsc::unbounded_channel();
        let mut painter = ConnectedPainter::new();
        let mut writer = Vec::new();
        let mut clipboard = ImageClipboard;
        let mut input = |event| {
            handle_input(
                event,
                &mut scopes,
                &mut screen,
                &commands,
                &mut writer,
                &mut clipboard,
                &mut painter,
            )
            .unwrap()
        };
        assert_eq!(
            input(key(event::KeyCode::Enter, event::KeyModifiers::NONE)),
            InputOutcome::Continue
        );
        assert!(
            matches!(requests.try_recv(), Ok(Request::Invoke { command, .. }) if command == "input.resolve")
        );
        assert!(
            requests.try_recv().is_err(),
            "Enter submitted the queued prompt"
        );
        assert_eq!(
            input(key(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)),
            InputOutcome::Quit
        );
        assert!(
            requests.try_recv().is_err(),
            "Quit submitted the queued prompt"
        );
        drop(input);
        assert!(screen.dialogs.focused());
        assert_eq!(scopes.active.uploads, 1);
        assert_eq!(scopes.active.pending[0].hash, "staged");
    }

    #[test]
    fn paste_and_resize_are_local_even_when_request_owner_is_busy() {
        let mut screen = ConnectedScreen::new(80, 24);
        let mut scopes = Scopes::new(&screen);
        let (commands, _requests) = mpsc::unbounded_channel();
        let mut painter = ConnectedPainter::new();
        let mut clipboard = ImageClipboard;
        let mut writer = Vec::new();
        for event in [Event::Paste("two\nlines".into()), Event::Resize(100, 30)] {
            assert_eq!(
                handle_input(
                    event,
                    &mut scopes,
                    &mut screen,
                    &commands,
                    &mut writer,
                    &mut clipboard,
                    &mut painter,
                )
                .unwrap(),
                InputOutcome::Continue
            );
        }
        assert_eq!(screen.ui.composer.text(), "two\nlines");
        assert_eq!((screen.ui.width, screen.ui.height), (100, 30));
    }
}
