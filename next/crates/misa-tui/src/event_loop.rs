//! Keyboard and local animation remain live while the session has nothing to send.
use crate::{KeyOut, Screen, Session};
use crate::{SessionReply, SessionRequest as Request};
use crossterm::event::{self, Event, KeyEventKind};
use std::io::Write;
#[cfg(test)]
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

fn offered_actions(
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
pub async fn run(session: &mut dyn Session) -> Result<(), String> {
    let mut screen = Screen::durable();
    screen.declare(&session.catalog());
    screen.location = session.location();
    if let Ok((width, height)) = crossterm::terminal::size() {
        screen.width = width;
        screen.height = height;
    }
    // Kitty support is advertised, never probed synchronously. A terminal that
    // reports its pixel geometry also tells us how big a cell is; otherwise the
    // default ratio is used.
    screen.graphics = misa_terminal_ui::graphics::Kitty::detect();
    if let Ok(size) = crossterm::terminal::window_size()
        && let Some(cell) = misa_terminal_ui::graphics::CellSize::from_window(
            screen.width,
            screen.height,
            size.width,
            size.height,
        )
    {
        screen.graphics.set_cell(cell);
    }
    let _terminal = crate::terminal_loop::Terminal::enter()?;
    let (_reader, receiver) = crate::terminal_loop::events();
    let result = drive(session, &mut screen, receiver, &mut std::io::stdout()).await;
    screen.save();
    result
}

async fn drive(
    session: &mut dyn Session,
    screen: &mut Screen,
    events: mpsc::Receiver<Result<Event, String>>,
    writer: &mut impl Write,
) -> Result<(), String> {
    drive_with_clipboard(
        session,
        screen,
        events,
        writer,
        &mut crate::clipboard::Desktop,
    )
    .await
}
async fn drive_with_clipboard(
    session: &mut dyn Session,
    screen: &mut Screen,
    mut events: mpsc::Receiver<Result<Event, String>>,
    writer: &mut impl Write,
    clipboard: &mut dyn crate::clipboard::Source,
) -> Result<(), String> {
    let mut retained =
        crate::retained::Retained::new(misa_proto::Node::section("session").id("session"), screen);
    let mut contributions = std::collections::BTreeMap::<String, crate::retained::Retained>::new();
    let mut pending = Vec::<misa_proto::view::BlobRef>::new();
    let mut uploads = 0usize;
    // Blobs whose bytes this client has asked for and not yet cached. Content
    // addressed, so the set spans scope switches.
    let mut requested = std::collections::HashSet::<String>::new();
    let mut images_dirty = true;
    let mut generation = 0u64;
    let mut scope = String::new();
    let mut parked = std::collections::BTreeMap::new();
    // The owner serializes work; the UI must not turn that serialization into dropped input.
    let (commands, requests) = mpsc::unbounded_channel();
    // A presentation is a fact the UI has to observe eventually. Letting the update pipe
    // backpressure the request owner is what turns a burst of startup notices into a full
    // input queue.
    let (updates, mut incoming) = mpsc::unbounded_channel();
    let driver = requests_loop(session, requests, updates);
    tokio::pin!(driver);
    let mut output = misa_terminal_ui::output::Output::default();
    let animations = misa_render::animations::Registry::stock();
    let mut animation = tokio::time::interval(Duration::from_millis(90));
    animation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame = 0usize;
    loop {
        // Fetch any image the tree names but the cache does not hold yet. This
        // walks once per document update, never per painted frame, and the
        // placeholder stays until the bytes arrive.
        if images_dirty {
            images_dirty = false;
            if screen.graphics.enabled() {
                let blobs: Vec<_> = retained
                    .image_blobs()
                    .into_iter()
                    .chain(contributions.values().flat_map(|c| c.image_blobs()))
                    .collect();
                for blob in blobs {
                    if screen.graphics.has(&blob.hash) || requested.contains(&blob.hash) {
                        continue;
                    }
                    requested.insert(blob.hash.clone());
                    enqueue(&commands, Request::Download { reference: blob }, screen);
                }
            }
        }
        let staging =
            (!pending.is_empty() || uploads > 0).then(|| match (uploads > 0, pending.is_empty()) {
                (true, true) => "Loading image…".to_string(),
                (true, false) => "Loading image…\nRemove last attachment".to_string(),
                (false, false) => "Remove last attachment".to_string(),
                (false, true) => unreachable!("staging exists only for uploads or attachments"),
            });
        let mut extra_document = vec![];
        let mut extra_footer = vec![];
        for contribution in contributions.values_mut() {
            contribution.local(screen);
            let (document, footer) = contribution.placed_lines(screen.height as usize);
            extra_document.extend(document);
            extra_footer.extend(footer);
        }
        screen.refresh_dialog_surface();
        let rendered =
            retained.frame_with(screen, staging.as_deref(), &extra_document, &extra_footer);
        // The viewport resolved a physical first row from the semantic anchor; keep
        // `scroll` at that row so the next reader delta is relative to what was shown,
        // and mirror back whether a scroll reached the tail and resumed following.
        screen.scroll = retained.resolved_scroll();
        screen.follow = retained.following();
        let mut rendered = rendered;
        let lines = &mut rendered.lines;
        if retained.has_turn() {
            // Activity owns the working animation. The frames are registered data;
            // the client only supplies the tick.
            if let Some(line) = lines
                .iter_mut()
                .find(|line| line.node.as_deref() == Some("indicators"))
            {
                let selected =
                    misa_lines::components::animation_for(screen.component_settings(), "activity")
                        .as_deref()
                        .and_then(|id| animations.frame(id, true, frame as u64));
                if let Some(selected) = selected
                    && let Some((_, text)) = line.spans.iter_mut().find(|(_, text)| text == "●")
                {
                    *text = selected.to_string();
                }
            }
        }
        crate::terminal_loop::paint(writer, &mut output, screen, rendered)?;
        let event = tokio::select! {
            result = &mut driver => return result,
            update = incoming.recv() => {
                match update {
                    Some(Update::View(crate::Presentation::Forget(id)))=>{
                        parked.remove(&id);
                        if scope==id {scope.clear();screen.composer.set_text("");screen.composer.clear_picker();screen.dialogs=Default::default();screen.panel=None;screen.selection=None;contributions.clear();pending.clear();uploads=0;}
                    },
                    Some(Update::View(crate::Presentation::Documents(documents))) => {
                        for (id,update) in documents {
                            if id.is_empty() {
                                crate::document_adapter::observed(&mut retained, &update,screen)?;
                            } else if let misa_client::document::Update::Unavailable(fault)=&update {
                                contributions.remove(&id);screen.notice=Some(format!("{id}: {}",fault.message));
                            } else {
                                let contribution=contributions.entry(id).or_insert_with(||crate::retained::Retained::new(misa_proto::Node::section("presentation").id("presentation"),screen));
                                crate::document_adapter::observed(contribution, &update,screen)?;
                            }
                        }
                    },
                    Some(Update::View(crate::Presentation::Activate(next))) => {
                        if scope.is_empty() {screen.enter_draft_scope(next.clone());scope=next;} else if scope != next {
                            screen.remember_draft();
                            let old = (
                                std::mem::replace(&mut retained, crate::retained::Retained::new(misa_proto::Node::section("session").id("session"), screen)),
                                std::mem::take(&mut contributions), std::mem::take(&mut screen.dialogs),
                                screen.composer.park(), screen.panel.take(),
                                screen.selection.take(), screen.scroll, screen.follow,
                                std::mem::take(&mut pending), uploads, generation,
                            );
                            if !scope.is_empty() { parked.insert(scope,old); }
                            if let Some((r,c,d,composer,panel,selection,scroll,follow,attachments,upload_count,epoch))=parked.remove(&next) {
                                retained=r;contributions=c;screen.dialogs=d;screen.composer.restore(composer);screen.panel=panel;screen.selection=selection;screen.scroll=scroll;screen.follow=follow;pending=attachments;uploads=upload_count;generation=epoch;screen.activate_draft_scope(next.clone());
                            } else { screen.restore_draft_scope(next.clone());screen.scroll=0;screen.follow=true;uploads=0;generation=generation.wrapping_add(1); }
                            scope=next;
                        }
                    },
                    Some(Update::View(crate::Presentation::Contribution { id, update })) => {
                        match &update {
                            misa_client::document::Update::Unavailable(fault) => { contributions.remove(&id); screen.notice = Some(format!("{id}: {}", fault.message)); },
                            _ => {
                                let contribution = contributions.entry(id).or_insert_with(|| crate::retained::Retained::new(misa_proto::Node::section("presentation").id("presentation"), screen));
                                crate::document_adapter::observed(contribution, &update, screen)?;
                            },
                        }
                    },
                    Some(Update::View(crate::Presentation::Attention{id,generation})) => {screen.dialogs.focus_request(id,generation);enqueue(&commands,Request::RefreshRequests,screen);},
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Request { id, generation, model }))) => screen.dialogs.update(id, generation, model),
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Report(report)))) => screen.dialogs.report(report),
                    Some(Update::View(crate::Presentation::Reply(SessionReply::DaemonForm{daemon,scope,form,drafts}))) => screen.dialogs.daemon_form(daemon,scope,form,drafts),
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Form(form)))) => screen.dialogs.form(form),
                    Some(Update::View(crate::Presentation::TurnOutput(_))) => {},
                    Some(Update::View(crate::Presentation::Declaration{catalog,location})) => {
                        screen.declare(&catalog); screen.location = location;
                    }
                    Some(Update::View(crate::Presentation::Candidates { source, items, truncated })) => screen.candidates(&source, items, truncated),
                    Some(Update::View(crate::Presentation::Snapshot(next))) => retained = crate::retained::Retained::new(next, screen),
                    Some(Update::View(crate::Presentation::Document(update))) => {
                        match &update {
                            misa_client::document::Update::Unavailable(fault) => screen.notice = Some(fault.message.clone()),
                            // Status is a semantic indicator rendered by the retained
                            // status owner. Turning it into a debug notice duplicates the
                            // status bar and makes normal activity look like an error.
                            misa_client::document::Update::Status(_) => {},
                            _ => {},
                        }
                        crate::document_adapter::observed(&mut retained, &update, screen)?;
                    }
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Complete { source, prefix, result }))) => match result {
                        Ok((items, truncated)) => {
                            screen.completion(&source, &prefix, items, truncated);
                        }
                        Err(error) => {
                            screen.completion_failed(&source);
                            screen.notice = Some(error);
                        }
                    },
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Notice(notice)))) => screen.notice = Some(notice),
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Uploaded { generation: reply_generation, result }))) => {
                        uploads = uploads.saturating_sub(1);
                        if generation == reply_generation {
                            match result { Ok(blob) => { pending.push(blob); screen.notice = Some("Image attached; Enter sends the prompt".into()); }, Err(error) => screen.notice = Some(error) }
                        }
                    }
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Downloaded { reference, result }))) => {
                        match result {
                            Ok(bytes) => match image::load_from_memory(&bytes) {
                                Ok(image) => screen.graphics.insert(&reference.hash, image.into_rgba8()),
                                Err(error) => screen.notice = Some(format!("Image decode failed: {error}")),
                            },
                            Err(error) => screen.notice = Some(error),
                        }
                    }
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Sent { draft, result }))) => match result {
                        Ok(()) => screen.notice = None,
                        Err(error) => {
                            if let Some((text, attachments)) = draft {
                                screen.composer.prepend(&text);
                                pending.extend(attachments);
                            }
                            screen.notice = Some(error);
                        }
                    },
                    None => return Ok(()),
                }
                images_dirty = true;
                continue;
            }
            _ = animation.tick(), if retained.has_turn() => {
                frame = frame.wrapping_add(1); continue;
            }
            event = events.recv() => match event { Some(event) => event?, None => return Ok(()) },
        };
        let view = retained.interaction();
        let out = match event {
            Event::Resize(width, height) => {
                screen.width = width;
                screen.height = height;
                retained.local(screen);
                output.invalidate();
                continue;
            }
            Event::Paste(text) => {
                if screen.dialogs.paste(&text) {
                } else if crate::panel_of(&view).is_some() {
                    for character in text.chars() {
                        screen.panel_key(&view, &crate::Key::Char(character));
                    }
                } else {
                    paste(screen, &text, &commands);
                }
                retained.local(screen);
                continue;
            }
            Event::Mouse(mouse) => {
                // The wheel is transcript scroll, three rows per notch, through the
                // same handler the keyboard scroll actions use.
                match mouse.kind {
                    event::MouseEventKind::ScrollUp => {
                        let _ = screen.key(crate::Key::ScrollPage(-3));
                    }
                    event::MouseEventKind::ScrollDown => {
                        let _ = screen.key(crate::Key::ScrollPage(3));
                    }
                    _ => continue,
                }
                retained.local(screen);
                continue;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == event::KeyCode::Char('v')
                    && key.modifiers.contains(event::KeyModifiers::CONTROL)
                {
                    if key.modifiers.contains(event::KeyModifiers::ALT) {
                        generation = generation.wrapping_add(1);
                        pending.clear();
                        screen.notice = Some("Clipboard attachments discarded".into());
                        continue;
                    }
                    match clipboard.read() {
                        Ok(crate::clipboard::Contents::Text(text)) => {
                            if screen.dialogs.paste(&text) {
                            } else if crate::panel_of(&view).is_some() {
                                for character in text.chars() {
                                    screen.panel_key(&view, &crate::Key::Char(character));
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
                                screen.notice = Some(
                                    "Close the input form to attach an image to the prompt".into(),
                                );
                                continue;
                            }
                            if pending.len() + uploads >= 16 {
                                screen.notice = Some(
                                    "Send or discard staged attachments before adding more".into(),
                                );
                                continue;
                            }
                            match crate::clipboard::png(width, height, rgba) {
                                Ok(bytes) => {
                                    if enqueue(
                                        &commands,
                                        Request::Upload {
                                            generation,
                                            bytes,
                                            media: "image/png".into(),
                                        },
                                        screen,
                                    ) {
                                        uploads += 1;
                                    }
                                }
                                Err(error) => screen.notice = Some(error),
                            }
                        }
                        Err(error) => screen.notice = Some(error),
                    }
                    retained.local(screen);
                    continue;
                }
                if key.code == event::KeyCode::Enter
                    && !screen.dialogs.focused()
                    && !key.modifiers.contains(event::KeyModifiers::SHIFT)
                    && uploads > 0
                {
                    screen.notice = Some("Wait for the attachment upload before sending".into());
                    continue;
                }
                let queued = view.children.iter().any(|node| node.id == "queue");
                if key.code == event::KeyCode::Enter
                    && !screen.dialogs.focused()
                    && screen.composer.text().is_empty()
                    && crate::panel_of(&view).is_none()
                    && !key.modifiers.contains(event::KeyModifiers::SHIFT)
                    && (queued || !pending.is_empty())
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
                    let Some(key) = crate::translate(key.code, key.modifiers, screen.keymap())
                    else {
                        continue;
                    };
                    // Ctrl-C has the reference's two meanings: while a turn is
                    // active it asks the owner to cancel; while the composer is
                    // otherwise idle it discards the local draft. The distinction
                    // belongs to this input router because only it has both the
                    // rendered session actions and the terminal-local draft.
                    if key == crate::Key::Interrupt
                        && !screen.composer.has_picker()
                        && !screen.dialogs.focused()
                        && screen.panel.is_none()
                        && screen.selection.is_none()
                        && !has_action(&view, "turn.cancel")
                    {
                        screen.composer.set_text("");
                        screen.notice = None;
                        continue;
                    }
                    if matches!(key, crate::Key::QueueEdit)
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
                        let dialog_keys = screen.dialog_settings().clone();
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
                                retained.local(screen);
                                continue;
                            }
                            None => {
                                crate::terminal_loop::route_key(screen, &mut retained, &view, key)
                            }
                        }
                    }
                }
            }
            _ => continue,
        };
        match out {
            KeyOut::Invoke { command, input } => {
                enqueue(&commands, Request::Invoke { command, input }, screen);
            }
            KeyOut::Local => {}
            KeyOut::Quit => return Ok(()),
            KeyOut::Intent(mut intent) => {
                if matches!(&intent,misa_kit::intent::Intent::Command{name,..} if name=="actions") {
                    let actions = offered_actions(&view, &contributions);
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
                    screen.composer.open_host_picker(picker, ":");
                    continue;
                }
                if let misa_kit::intent::Intent::Command { name, args } = &intent
                    && name == "action"
                {
                    let actions = offered_actions(&view, &contributions);
                    let id = args
                        .get("action")
                        .and_then(misa_value::Value::as_str)
                        .unwrap_or("");
                    let Some((node, _, _)) = actions.get(id) else {
                        screen.notice =
                            Some("Action is no longer offered by a visible document".into());
                        continue;
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
                    continue;
                }
                let draft = match &mut intent {
                    misa_kit::intent::Intent::Prompt { text, attachments }
                    | misa_kit::intent::Intent::Interrupt { text, attachments } => {
                        attachments.extend(pending.iter().cloned());
                        Some(text.clone())
                    }
                    _ => None,
                };
                if enqueue(&commands, Request::Intent(intent), screen) {
                    if draft.is_some() {
                        pending.clear();
                    }
                } else if let Some(text) = draft {
                    screen.composer.set_text(text);
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
                    Err(error) => screen.notice = Some(error),
                },
                Some(Err(error)) => {
                    screen.composer.set_text(text);
                    screen.save();
                    screen.notice = Some(error);
                }
                None => screen.notice = Some("Unknown host command".into()),
            },
            KeyOut::Copy(text) => {
                use base64::Engine as _;
                let encoded = base64::engine::general_purpose::STANDARD.encode(text);
                write!(writer, "\x1b]52;c;{encoded}\x07").map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
            }
            KeyOut::StartSelection => {
                // Selection needs the retained document's rendered geometry;
                // keep that operation at the client/view boundary rather than
                // smuggling a second document copy into Screen::action.
                let _ = retained.selection_key(screen, &crate::Key::StartSelection);
            }
        }
        retained.local(screen);
    }
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

// The session is borrowed by this future, not by the keyboard branch. Dropping the drive
// future cancels outstanding UI waits.
enum Update {
    View(crate::Presentation),
}
fn paste(screen: &mut Screen, text: &str, commands: &mpsc::UnboundedSender<Request>) {
    if let KeyOut::Complete { source, prefix } = screen.paste(text) {
        enqueue(commands, Request::Complete { source, prefix }, screen);
    }
}
fn enqueue(sender: &mpsc::UnboundedSender<Request>, request: Request, screen: &mut Screen) -> bool {
    if sender.send(request).is_err() {
        screen.notice = Some("The session request owner is closed".into());
        false
    } else {
        true
    }
}
async fn requests_loop(
    session: &mut dyn Session,
    mut requests: mpsc::UnboundedReceiver<Request>,
    updates: mpsc::UnboundedSender<Update>,
) -> Result<(), String> {
    loop {
        let update = tokio::select! {
            request = requests.recv() => match request {
                None => return Ok(()),
                Some(request) => match session.request(request).await {
                    Some(reply) => Update::View(crate::Presentation::Reply(reply)), None => continue,
                },
            },
            incoming = session.next_presentation() => match incoming? {
                Some(view) => Update::View(view), None => return Ok(()),
            },
        };
        if updates.send(update).is_err() {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_kit::intent::Intent;
    use misa_proto::Node;
    use misa_proto::view::Choice;
    struct Idle {
        first: bool,
    }
    #[async_trait::async_trait]
    impl Session for Idle {
        async fn next(&mut self) -> Result<Option<Node>, String> {
            if self.first {
                self.first = false;
                return Ok(Some(Node::section("session").id("session")));
            }
            std::future::pending().await
        }
        async fn send(&mut self, _: Intent) -> Result<(), String> {
            Ok(())
        }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> {
            Ok((vec![], false))
        }
    }
    #[tokio::test]
    async fn an_idle_session_still_accepts_paste_resize_and_quit() {
        let mut session = Idle { first: true };
        let mut screen = Screen::new(80, 24);
        let (sender, receiver) = mpsc::channel(64);
        sender
            .try_send(Ok(Event::Paste("two\nlines".into())))
            .unwrap();
        sender.try_send(Ok(Event::Resize(100, 30))).unwrap();
        sender
            .try_send(Ok(Event::Key(event::KeyEvent::new(
                event::KeyCode::Char('q'),
                event::KeyModifiers::CONTROL,
            ))))
            .unwrap();
        let mut output = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(1),
            drive(&mut session, &mut screen, receiver, &mut output),
        )
        .await
        .expect("idle session blocked local input")
        .unwrap();
        assert_eq!(screen.composer.text(), "two\nlines");
        assert_eq!((screen.width, screen.height), (100, 30));
    }

    struct Waiting {
        started: Arc<tokio::sync::Notify>,
    }
    #[async_trait::async_trait]
    impl Session for Waiting {
        async fn next(&mut self) -> Result<Option<Node>, String> {
            std::future::pending().await
        }
        async fn send(&mut self, _: Intent) -> Result<(), String> {
            std::future::pending().await
        }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> {
            self.started.notify_one();
            std::future::pending().await
        }
        async fn save_attachment(&mut self, _: &str, _: &str) -> Result<(), String> {
            self.started.notify_one();
            std::future::pending().await
        }
    }
    #[tokio::test]
    async fn a_slow_completion_keeps_paste_resize_and_quit_live() {
        let started = Arc::new(tokio::sync::Notify::new());
        let mut session = Waiting {
            started: started.clone(),
        };
        let mut screen = Screen::new(80, 24);
        screen.declare(&misa_tui_ui::Catalog {
            commands: vec![
                misa_kit::intent::Command::new("model", "Model", "choose").arg(
                    misa_proto::preparation::Arg::new("model", "Model")
                        .required()
                        .from("models"),
                ),
            ],
            sources: vec![misa_kit::intent::Source::resident("models", "Models")],
        });
        screen.composer.set_text("/model");
        let (sender, receiver) = mpsc::channel(64);
        sender
            .try_send(Ok(Event::Key(event::KeyEvent::new(
                event::KeyCode::Tab,
                event::KeyModifiers::NONE,
            ))))
            .unwrap();
        let input = async move {
            started.notified().await;
            sender
                .try_send(Ok(Event::Paste("still editing".into())))
                .unwrap();
            sender.try_send(Ok(Event::Resize(100, 30))).unwrap();
            sender
                .try_send(Ok(Event::Key(event::KeyEvent::new(
                    event::KeyCode::Char('q'),
                    event::KeyModifiers::CONTROL,
                ))))
                .unwrap();
        };
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), async {
            let (result, ()) = tokio::join!(
                drive(&mut session, &mut screen, receiver, &mut output),
                input
            );
            result.unwrap();
        })
        .await
        .expect("completion blocked local input");
        assert!(screen.composer.text().contains("still editing"));
        assert_eq!((screen.width, screen.height), (100, 30));
    }

    #[tokio::test]
    async fn startup_does_not_wait_for_a_snapshot_before_quit() {
        let mut session = Waiting {
            started: Default::default(),
        };
        let mut screen = Screen::new(80, 24);
        let (sender, receiver) = mpsc::channel(64);
        sender
            .try_send(Ok(Event::Key(event::KeyEvent::new(
                event::KeyCode::Char('q'),
                event::KeyModifiers::CONTROL,
            ))))
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            drive(&mut session, &mut screen, receiver, &mut Vec::new()),
        )
        .await
        .expect("startup blocked quit")
        .unwrap();
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use misa_kit::intent::Intent;
    use misa_proto::Node;
    use misa_proto::view::{BlobRef, Choice};
    use std::sync::atomic::AtomicUsize;
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
    struct FrameWriter {
        bytes: Vec<u8>,
        frame: Arc<AtomicUsize>,
        changed: tokio::sync::watch::Sender<usize>,
    }
    impl Write for FrameWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            let frame = self.frame.fetch_add(1, Ordering::SeqCst) + 1;
            self.changed.send_replace(frame);
            Ok(())
        }
    }
    struct UploadSession {
        fail: bool,
        uploads: usize,
        sent: Vec<Intent>,
        frame: Arc<AtomicUsize>,
        receipts: mpsc::Sender<usize>,
    }
    #[async_trait::async_trait]
    impl Session for UploadSession {
        async fn next(&mut self) -> Result<Option<Node>, String> {
            std::future::pending().await
        }
        async fn send(&mut self, intent: Intent) -> Result<(), String> {
            self.sent.push(intent);
            self.receipts
                .send(self.frame.load(Ordering::SeqCst))
                .await
                .unwrap();
            if self.fail {
                self.fail = false;
                Err("retry send".into())
            } else {
                Ok(())
            }
        }
        async fn upload(&mut self, bytes: Vec<u8>, media: &str) -> Result<BlobRef, String> {
            assert_eq!(media, "image/png");
            assert_eq!(
                image::load_from_memory(&bytes)
                    .unwrap()
                    .into_rgba8()
                    .into_raw(),
                vec![255, 0, 0, 255]
            );
            self.uploads += 1;
            self.receipts
                .send(self.frame.load(Ordering::SeqCst))
                .await
                .unwrap();
            Ok(BlobRef {
                hash: format!("blob-{}", self.uploads),
                len: bytes.len() as u64,
                media: Some(media.into()),
            })
        }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> {
            Ok((vec![], false))
        }
    }
    fn key(code: event::KeyCode, modifiers: event::KeyModifiers) -> Result<Event, String> {
        Ok(Event::Key(event::KeyEvent::new(code, modifiers)))
    }
    async fn acknowledged(
        receipts: &mut mpsc::Receiver<usize>,
        frames: &mut tokio::sync::watch::Receiver<usize>,
    ) {
        let before = receipts.recv().await.unwrap();
        while *frames.borrow_and_update() <= before {
            frames.changed().await.unwrap();
        }
    }
    #[tokio::test]
    async fn staged_images_retry_exactly_and_discard_without_a_prompt() {
        let frame = Arc::new(AtomicUsize::new(0));
        let (changed, mut frames) = tokio::sync::watch::channel(0);
        let (receipts, mut received) = mpsc::channel(8);
        let mut session = UploadSession {
            fail: true,
            uploads: 0,
            sent: vec![],
            frame: frame.clone(),
            receipts,
        };
        let mut writer = FrameWriter {
            bytes: vec![],
            frame,
            changed,
        };
        let mut screen = Screen::new(80, 24);
        screen.composer.set_text("a picture");
        let (sender, receiver) = mpsc::channel(64);
        let script = async move {
            sender
                .try_send(key(event::KeyCode::Char('v'), event::KeyModifiers::CONTROL))
                .unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender
                .try_send(key(event::KeyCode::Enter, event::KeyModifiers::NONE))
                .unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender
                .try_send(key(event::KeyCode::Enter, event::KeyModifiers::NONE))
                .unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender
                .try_send(key(event::KeyCode::Char('v'), event::KeyModifiers::CONTROL))
                .unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender
                .try_send(key(
                    event::KeyCode::Char('v'),
                    event::KeyModifiers::CONTROL | event::KeyModifiers::ALT,
                ))
                .unwrap();
            sender
                .try_send(key(event::KeyCode::Enter, event::KeyModifiers::NONE))
                .unwrap();
            sender
                .try_send(key(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL))
                .unwrap();
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut clipboard = ImageClipboard;
            let (result, ()) = tokio::join!(
                drive_with_clipboard(
                    &mut session,
                    &mut screen,
                    receiver,
                    &mut writer,
                    &mut clipboard
                ),
                script
            );
            result.unwrap();
        })
        .await
        .expect("clipboard requests stalled");
        assert_eq!(session.uploads, 2);
        assert_eq!(session.sent.len(), 2);
        assert_eq!(session.sent[0], session.sent[1]);
        assert!(
            matches!(&session.sent[1], Intent::Prompt { text, attachments } if text == "a picture" && attachments.len() == 1 && attachments[0].hash == "blob-1")
        );
        assert!(screen.composer.text().is_empty());
        assert!(
            String::from_utf8(writer.bytes)
                .unwrap()
                .contains("Remove last attachment")
        );
    }
    #[tokio::test]
    async fn image_only_alt_enter_carries_the_staged_ref() {
        let frame = Arc::new(AtomicUsize::new(0));
        let (changed, mut frames) = tokio::sync::watch::channel(0);
        let (receipts, mut received) = mpsc::channel(8);
        let mut session = UploadSession {
            fail: false,
            uploads: 0,
            sent: vec![],
            frame: frame.clone(),
            receipts,
        };
        let mut writer = FrameWriter {
            bytes: vec![],
            frame,
            changed,
        };
        let mut screen = Screen::new(80, 24);
        let (sender, receiver) = mpsc::channel(64);
        let script = async move {
            sender
                .try_send(key(event::KeyCode::Char('v'), event::KeyModifiers::CONTROL))
                .unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender
                .try_send(key(event::KeyCode::Enter, event::KeyModifiers::ALT))
                .unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender
                .try_send(key(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL))
                .unwrap();
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut clipboard = ImageClipboard;
            let (result, ()) = tokio::join!(
                drive_with_clipboard(
                    &mut session,
                    &mut screen,
                    receiver,
                    &mut writer,
                    &mut clipboard
                ),
                script
            );
            result.unwrap();
        })
        .await
        .unwrap();
        assert!(
            matches!(&session.sent[..], [Intent::Interrupt { text, attachments }] if text.is_empty() && attachments.len() == 1)
        );
    }
}

#[cfg(test)]
mod save_liveness_test {
    use super::*;
    use misa_kit::intent::Intent;
    use misa_proto::Node;
    use misa_proto::view::{Action, ActionOn, Choice};
    struct Saving {
        first: bool,
        started: Arc<tokio::sync::Notify>,
    }
    #[async_trait::async_trait]
    impl Session for Saving {
        async fn next(&mut self) -> Result<Option<Node>, String> {
            if self.first {
                self.first = false;
                return Ok(Some(Node::section("session").id("session").child(
                    Node::section("attachment").id("file").action(Action {
                        id: "attachment.save".into(),
                        on: ActionOn::Click,
                        label: None,
                        args: misa_value::Value::Null,
                    }),
                )));
            }
            std::future::pending().await
        }
        async fn send(&mut self, _: Intent) -> Result<(), String> {
            Ok(())
        }
        async fn save_attachment(&mut self, node: &str, _: &str) -> Result<(), String> {
            assert_eq!(node, "file");
            self.started.notify_one();
            std::future::pending().await
        }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> {
            Ok((vec![], false))
        }
    }
    struct Observed {
        bytes: Vec<u8>,
        ready: Arc<tokio::sync::Notify>,
    }
    impl Write for Observed {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if String::from_utf8_lossy(&self.bytes).contains("misa") {
                self.ready.notify_one();
            }
            Ok(())
        }
    }
    #[tokio::test]
    async fn waiting_for_a_save_keeps_edit_resize_and_quit_live() {
        let ready = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(tokio::sync::Notify::new());
        let mut session = Saving {
            first: true,
            started: started.clone(),
        };
        let mut screen = Screen::new(80, 24);
        screen.declare(&session.catalog());
        screen.composer.set_text("/save /tmp/unused-save-test");
        let mut writer = Observed {
            bytes: vec![],
            ready: ready.clone(),
        };
        let (sender, receiver) = mpsc::channel(64);
        let input = async move {
            ready.notified().await;
            sender
                .try_send(Ok(Event::Key(event::KeyEvent::new(
                    event::KeyCode::Enter,
                    event::KeyModifiers::NONE,
                ))))
                .unwrap();
            started.notified().await;
            sender
                .try_send(Ok(Event::Paste("still editing".into())))
                .unwrap();
            sender.try_send(Ok(Event::Resize(90, 30))).unwrap();
            sender
                .try_send(Ok(Event::Key(event::KeyEvent::new(
                    event::KeyCode::Char('q'),
                    event::KeyModifiers::CONTROL,
                ))))
                .unwrap();
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            let (result, ()) = tokio::join!(
                drive(&mut session, &mut screen, receiver, &mut writer),
                input
            );
            result.unwrap();
        })
        .await
        .expect("save wait blocked keyboard");
        assert_eq!(screen.composer.text(), "still editing");
        assert_eq!((screen.width, screen.height), (90, 30));
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    struct SurfaceSession {
        updates: mpsc::Receiver<crate::Presentation>,
        received: mpsc::Sender<usize>,
        frames: Arc<std::sync::atomic::AtomicUsize>,
        sent: Option<mpsc::Sender<misa_kit::intent::Intent>>,
    }
    #[async_trait::async_trait]
    impl Session for SurfaceSession {
        async fn complete(
            &mut self,
            _source: &str,
            _prefix: &str,
        ) -> Result<(Vec<misa_proto::view::Choice>, bool), String> {
            Ok((vec![], false))
        }
        async fn next_presentation(&mut self) -> Result<Option<crate::Presentation>, String> {
            let next = self.updates.recv().await;
            if next.is_some() {
                let _ = self.received.send(self.frames.load(Ordering::SeqCst)).await;
            }
            Ok(next)
        }
        async fn next(&mut self) -> Result<Option<misa_proto::Node>, String> {
            std::future::pending().await
        }
        async fn send(&mut self, intent: misa_kit::intent::Intent) -> Result<(), String> {
            if let Some(sent) = &self.sent {
                sent.send(intent).await.unwrap();
            }
            Ok(())
        }
    }
    struct Writer {
        frames: Arc<std::sync::atomic::AtomicUsize>,
        changed: tokio::sync::watch::Sender<usize>,
    }
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.changed
                .send_replace(self.frames.fetch_add(1, Ordering::SeqCst) + 1);
            Ok(())
        }
    }
    async fn painted(frames: &mut tokio::sync::watch::Receiver<usize>, after: usize) {
        while *frames.borrow_and_update() <= after {
            frames.changed().await.unwrap();
        }
    }
    #[tokio::test]
    async fn scope_switch_preserves_composer_and_hidden_private_draft() {
        let frame = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (changed, mut frames) = tokio::sync::watch::channel(0);
        let (updates, receive_updates) = mpsc::channel(8);
        let (received, mut receipts) = mpsc::channel(8);
        let mut session = SurfaceSession {
            updates: receive_updates,
            received,
            frames: frame.clone(),
            sent: None,
        };
        let mut writer = Writer {
            frames: frame.clone(),
            changed,
        };
        let mut screen = Screen::new(80, 24);
        screen.set_draft_for("A".into(), "draft A".into());
        screen.dialogs.update(
            "secret".into(),
            1,
            Some(misa_client::request::Model {
                form: None,
                id: "secret".into(),
                generation: 1,
                title: "Credential".into(),
                body: misa_proto::Node::section("request").id("request"),
                input: Some(misa_client::request::Input {
                    id: "value".into(),
                    label: "Key".into(),
                    secret: true,
                }),
                actions: vec![],
            }),
        );
        screen.dialogs.open();
        screen
            .dialogs
            .key(&crate::Key::Char('s'), &screen.dialog_settings().clone());
        screen
            .dialogs
            .key(&crate::Key::Escape, &screen.dialog_settings().clone());
        let (keys, events) = mpsc::channel(8);
        let script = async move {
            for scope in ["A", "B"] {
                updates
                    .send(crate::Presentation::Activate(scope.into()))
                    .await
                    .unwrap();
                painted(&mut frames, receipts.recv().await.unwrap()).await;
            }
            let before = frame.load(Ordering::SeqCst);
            keys.send(Ok(Event::Paste("draft B".into()))).await.unwrap();
            painted(&mut frames, before).await;
            updates
                .send(crate::Presentation::Activate("A".into()))
                .await
                .unwrap();
            painted(&mut frames, receipts.recv().await.unwrap()).await;
            keys.send(Ok(Event::Key(event::KeyEvent::new(
                event::KeyCode::Char('q'),
                event::KeyModifiers::CONTROL,
            ))))
            .await
            .unwrap();
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            let (result, ()) = tokio::join!(
                drive(&mut session, &mut screen, events, &mut writer),
                script
            );
            result.unwrap();
        })
        .await
        .unwrap();
        assert_eq!(screen.composer.text(), "draft A");
        screen.dialogs.open();
        screen
            .dialogs
            .key(&crate::Key::Char('t'), &screen.dialog_settings().clone());
        let text = misa_lines::to_plain(&screen.dialogs.lines(
            &screen.theme,
            80,
            screen.dialog_settings(),
        ));
        assert!(text.contains("••"), "hidden secret draft was lost: {text}");
    }
    #[tokio::test]
    async fn returning_to_parked_picker_keeps_the_latest_declaration_and_resident_items() {
        use misa_kit::intent::{Command, Source};
        use misa_proto::preparation::Arg;
        use misa_proto::view::Choice;

        let frame = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (changed, mut frames) = tokio::sync::watch::channel(0);
        let (updates, receive_updates) = mpsc::channel(8);
        let (received, mut receipts) = mpsc::channel(8);
        let mut session = SurfaceSession {
            updates: receive_updates,
            received,
            frames: frame.clone(),
            sent: None,
        };
        let mut writer = Writer {
            frames: frame,
            changed,
        };
        let mut screen = Screen::new(80, 24);
        let original = misa_tui_ui::Catalog {
            commands: vec![
                Command::new("model", "Model", "choose")
                    .arg(Arg::new("model", "Model").required().from("models")),
            ],
            sources: vec![Source::resident("models", "Models")],
        };
        screen.declare(&original);
        screen.composer.set_text("/model");
        assert!(matches!(
            screen.key(crate::Key::Submit),
            KeyOut::Complete { .. }
        ));
        screen.key(crate::Key::Char('f'));
        screen.set_draft_for("A".into(), "/model f".into());
        let mut latest = original.clone();
        latest
            .commands
            .push(Command::new("fresh", "Fresh", "new command"));
        let (keys, events) = mpsc::channel(8);
        let script = async move {
            for scope in ["A", "B"] {
                updates
                    .send(crate::Presentation::Activate(scope.into()))
                    .await
                    .unwrap();
                painted(&mut frames, receipts.recv().await.unwrap()).await;
            }
            updates
                .send(crate::Presentation::Declaration {
                    catalog: latest,
                    location: String::new(),
                })
                .await
                .unwrap();
            painted(&mut frames, receipts.recv().await.unwrap()).await;
            updates
                .send(crate::Presentation::Candidates {
                    source: "models".into(),
                    items: vec![Choice {
                        value: "fresh-model".into(),
                        label: "Fresh model".into(),
                        detail: None,
                        metadata: None,
                    }],
                    truncated: false,
                })
                .await
                .unwrap();
            painted(&mut frames, receipts.recv().await.unwrap()).await;
            updates
                .send(crate::Presentation::Activate("A".into()))
                .await
                .unwrap();
            painted(&mut frames, receipts.recv().await.unwrap()).await;
            keys.send(Ok(Event::Key(event::KeyEvent::new(
                event::KeyCode::Char('q'),
                event::KeyModifiers::CONTROL,
            ))))
            .await
            .unwrap();
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            let (result, ()) = tokio::join!(
                drive(&mut session, &mut screen, events, &mut writer),
                script
            );
            result.unwrap();
        })
        .await
        .unwrap();
        assert_eq!(screen.composer.text(), "/model f");
        assert_eq!(screen.composer.picker().unwrap().query, "f");
        assert!(
            screen
                .command_candidates()
                .iter()
                .any(|c| c.value == "/fresh")
        );
        screen.key(crate::Key::Escape);
        assert_eq!(
            screen.key(crate::Key::Action(crate::Action::OpenModel)),
            KeyOut::Local
        );
        assert_eq!(
            screen
                .composer
                .picker()
                .unwrap()
                .selected()
                .map(|c| c.value.as_str()),
            Some("fresh-model")
        );
    }

    #[test]
    fn optional_documents_contribute_portable_actions_without_merging_node_spaces() {
        let screen = Screen::new(80, 24);
        let root = misa_proto::Node::section("conversation").id("same");
        let mut pet = misa_proto::Node::section("pet").id("same");
        pet.actions.push(misa_proto::view::Action {
            id: "pet.feed".into(),
            label: Some("Feed pet".into()),
            on: Default::default(),
            args: misa_value::Value::Null,
        });
        let mut documents = std::collections::BTreeMap::from([(
            "plugin.pet.companion".into(),
            crate::retained::Retained::new(pet, &screen),
        )]);
        let actions = offered_actions(&root, &documents);
        assert_eq!(
            actions["pet.feed"],
            (
                "same".into(),
                "Feed pet".into(),
                "plugin.pet.companion".into()
            )
        );
        documents.clear();
        assert!(
            !offered_actions(&root, &documents).contains_key("pet.feed"),
            "hidden documents cannot leave active action candidates"
        );
    }
    #[tokio::test]
    async fn optional_document_action_chooser_invokes_its_offered_binding() {
        let frame = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (changed, mut frames) = tokio::sync::watch::channel(0);
        let (updates, receive_updates) = mpsc::channel(8);
        let (received, mut receipts) = mpsc::channel(8);
        let (sent, mut intents) = mpsc::channel(8);
        let mut session = SurfaceSession {
            updates: receive_updates,
            received,
            frames: frame.clone(),
            sent: Some(sent),
        };
        let mut writer = Writer {
            frames: frame.clone(),
            changed,
        };
        let mut screen = Screen::new(80, 24);
        screen.declare(&misa_tui_ui::Catalog {
            commands: vec![
                misa_kit::intent::Command::new("actions", "Actions", "Actions"),
                misa_kit::intent::Command::new("action", "Action", "Action")
                    .arg(misa_proto::preparation::Arg::new("action", "Action").required()),
            ],
            sources: vec![],
        });
        screen.composer.set_text("/actions");
        let mut pet = misa_proto::Node::section("pet").id("pet");
        pet.actions.push(misa_proto::view::Action {
            id: "pet.feed".into(),
            label: Some("Feed pet".into()),
            on: Default::default(),
            args: misa_value::Value::Null,
        });
        let (keys, events) = mpsc::channel(8);
        let script = async move {
            updates
                .send(crate::Presentation::Documents(vec![(
                    "plugin.pet.companion".into(),
                    misa_client::document::Update::Reset(misa_proto::observation::Document {
                        version: misa_proto::sync::Version {
                            epoch: "pet".into(),
                            rev: 0,
                        },
                        tree: pet,
                        streams: vec![],
                    }),
                )]))
                .await
                .unwrap();
            painted(&mut frames, receipts.recv().await.unwrap()).await;
            let before = frame.load(Ordering::SeqCst);
            keys.send(Ok(Event::Key(event::KeyEvent::new(
                event::KeyCode::Enter,
                event::KeyModifiers::NONE,
            ))))
            .await
            .unwrap();
            painted(&mut frames, before).await;
            keys.send(Ok(Event::Key(event::KeyEvent::new(
                event::KeyCode::Enter,
                event::KeyModifiers::NONE,
            ))))
            .await
            .unwrap();
            assert!(
                matches!(intents.recv().await.unwrap(),misa_kit::intent::Intent::Action{node,action,..} if node=="pet" && action=="pet.feed")
            );
            keys.send(Ok(Event::Key(event::KeyEvent::new(
                event::KeyCode::Char('q'),
                event::KeyModifiers::CONTROL,
            ))))
            .await
            .unwrap();
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            let (result, ()) = tokio::join!(
                drive(&mut session, &mut screen, events, &mut writer),
                script
            );
            result.unwrap();
        })
        .await
        .expect("optional document action was not usable");
    }
}
