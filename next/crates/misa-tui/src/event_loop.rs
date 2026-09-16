//! Keyboard and local animation remain live while the session has nothing to send.
use std::io::Write;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc;
use crate::{KeyOut, Screen, Session};
use crate::{SessionRequest as Request, SessionReply};

struct Terminal;
fn offered_actions(view:&misa_proto::Node,contributions:&std::collections::BTreeMap<String,crate::retained::Retained>)->std::collections::BTreeMap<String,(String,String,String)> {
    fn visit(node:&misa_proto::Node,origin:&str,actions:&mut std::collections::BTreeMap<String,(String,String,String)>){
        for action in &node.actions {actions.insert(action.id.clone(),(node.id.clone(),action.label.clone().unwrap_or_else(||action.id.clone()),origin.into()));}
        for child in &node.children{visit(child,origin,actions);}
        if let misa_proto::view::Kind::List{items,..}=&node.kind {for item in items{for child in item{visit(child,origin,actions);}}}
    }
    let mut actions=std::collections::BTreeMap::new();
    visit(view,"Conversation",&mut actions);
    for (id,document) in contributions{visit(&document.interaction(),id,&mut actions);}
    actions
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste,
            crossterm::terminal::LeaveAlternateScreen);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
struct Reader { stop: Arc<AtomicBool>, thread: Option<std::thread::JoinHandle<()>> }
impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

pub async fn run(session: &mut dyn Session) -> Result<(), String> {
    let mut screen = Screen::durable();
    screen.declare(&session.catalog()); screen.location = session.location();
    if let Ok((width, height)) = crossterm::terminal::size() { screen.width = width; screen.height = height; }
    crossterm::terminal::enable_raw_mode().map_err(|error| error.to_string())?;
    let _terminal = Terminal;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen,
        event::EnableBracketedPaste).map_err(|error| error.to_string())?;
    let (sender, receiver) = mpsc::channel(64);
    let stop = Arc::new(AtomicBool::new(false));
    let reading = stop.clone();
    let thread = std::thread::spawn(move || {
        while !reading.load(Ordering::Relaxed) {
            let mut pending = match event::poll(Duration::from_millis(50)) {
                Ok(false) => continue,
                Ok(true) => event::read().map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            loop {
                if reading.load(Ordering::Relaxed) { return; }
                match sender.try_send(pending) {
                    Ok(()) => break,
                    Err(mpsc::error::TrySendError::Full(event)) => { pending = event; std::thread::sleep(Duration::from_millis(5)); }
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                }
            }
        }
    });
    let _reader = Reader { stop, thread: Some(thread) };
    let result = drive(session, &mut screen, receiver, &mut std::io::stdout()).await;
    screen.save();
    result
}

async fn drive(session: &mut dyn Session, screen: &mut Screen,
    events: mpsc::Receiver<Result<Event, String>>, writer: &mut impl Write) -> Result<(), String> {
    drive_with_clipboard(session, screen, events, writer, &mut crate::clipboard::Desktop).await
}
async fn drive_with_clipboard(session: &mut dyn Session, screen: &mut Screen,
    mut events: mpsc::Receiver<Result<Event, String>>, writer: &mut impl Write,
    clipboard: &mut dyn crate::clipboard::Source) -> Result<(), String> {
    let mut retained = crate::retained::Retained::new(misa_proto::Node::section("session").id("session"), screen);
    let mut contributions = std::collections::BTreeMap::<String, crate::retained::Retained>::new();
    let mut pending = Vec::<misa_proto::view::BlobRef>::new();
    let mut uploads = 0usize;
    let mut generation = 0u64;
    let mut scope = String::new();
    let mut parked = std::collections::BTreeMap::new();
    let (commands, requests) = mpsc::channel(16);
    let (updates, mut incoming) = mpsc::channel(1);
    let driver = requests_loop(session, requests, updates);
    tokio::pin!(driver);
    let mut output = crate::output::Output::default();
    let mut animation = tokio::time::interval(Duration::from_millis(90));
    animation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame = 0usize;
    loop {
        let staging = (!pending.is_empty() || uploads > 0).then(|| format!("{} clipboard attachments · {uploads} uploading · Ctrl-Alt-V discards", pending.len()));
        let mut extra_document = vec![];
        let mut extra_footer = vec![];
        for contribution in contributions.values_mut() {
            contribution.local(screen);
            let (document, footer) = contribution.placed_lines(screen.height as usize);
            extra_document.extend(document); extra_footer.extend(footer);
        }
        let rendered = retained.frame_with(screen, staging.as_deref(), &extra_document, &extra_footer);
        let mut lines = rendered.lines;
        if retained.has_turn() {
            const FRAMES: [&str; 4] = ["⠋", "⠙", "⠹", "⠸"];
            if let Some(line) = lines.iter_mut().find(|line| line.node.as_deref() == Some("turn")) {
                line.spans.insert(0, (screen.theme.role("turn"), format!("{} ", FRAMES[frame % FRAMES.len()])));
            }
        }
        output.paint(writer, &lines).map_err(|error| error.to_string())?;
        write!(writer, "\x1b[{};{}H", rendered.cursor_row + 1, rendered.cursor_column + 1)
            .and_then(|_| writer.flush()).map_err(|error| error.to_string())?;
        let event = tokio::select! {
            result = &mut driver => return result,
            update = incoming.recv() => {
                match update {
                    Some(Update::View(crate::Presentation::Forget(id)))=>{
                        parked.remove(&id);
                        if scope==id {scope.clear();screen.editor.set_text("");screen.dialogs=Default::default();screen.picker=None;screen.pending_command=None;screen.panel=None;screen.selection=None;contributions.clear();pending.clear();uploads=0;}
                    },
                    Some(Update::View(crate::Presentation::Documents(documents))) => {
                        for (id,update) in documents {
                            if id.is_empty() {
                                retained.observed(&update,screen)?;
                            } else if let misa_client::document::Update::Unavailable(fault)=&update {
                                contributions.remove(&id);screen.notice=Some(format!("{id}: {}",fault.message));
                            } else {
                                let contribution=contributions.entry(id).or_insert_with(||crate::retained::Retained::new(misa_proto::Node::section("presentation").id("presentation"),screen));
                                contribution.observed(&update,screen)?;
                            }
                        }
                    },
                    Some(Update::View(crate::Presentation::Activate(next))) => {
                        if scope.is_empty() {screen.enter_draft_scope(next.clone());scope=next;} else if scope != next {
                            screen.remember_draft();
                            let old = (
                                std::mem::replace(&mut retained, crate::retained::Retained::new(misa_proto::Node::section("session").id("session"), screen)),
                                std::mem::take(&mut contributions), std::mem::take(&mut screen.dialogs),
                                std::mem::replace(&mut screen.editor, crate::ed::Editor::new()),
                                screen.picker.take(), screen.pending_command.take(), screen.panel.take(),
                                screen.selection.take(), screen.scroll, screen.follow,
                                std::mem::take(&mut pending), uploads, generation,
                            );
                            if !scope.is_empty() { parked.insert(scope,old); }
                            if let Some((r,c,d,e,p,command,panel,selection,scroll,follow,attachments,upload_count,epoch))=parked.remove(&next) {
                                retained=r;contributions=c;screen.dialogs=d;screen.editor=e;screen.picker=p;screen.pending_command=command;screen.panel=panel;screen.selection=selection;screen.scroll=scroll;screen.follow=follow;pending=attachments;uploads=upload_count;generation=epoch;
                            } else { screen.editor.set_text(screen.prefs.drafts.get(&next).cloned().unwrap_or_default());screen.scroll=0;screen.follow=true;uploads=0;generation=generation.wrapping_add(1); }
                            screen.draft_scope=Some(next.clone());
                            scope=next;
                        }
                    },
                    Some(Update::View(crate::Presentation::Contribution { id, update })) => {
                        match &update {
                            misa_client::document::Update::Unavailable(fault) => { contributions.remove(&id); screen.notice = Some(format!("{id}: {}", fault.message)); },
                            _ => {
                                let contribution = contributions.entry(id).or_insert_with(|| crate::retained::Retained::new(misa_proto::Node::section("presentation").id("presentation"), screen));
                                contribution.observed(&update, screen)?;
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
                            misa_client::document::Update::Status(status) => screen.notice = Some(format!("Session: {status:?}")),
                            _ => {},
                        }
                        retained.observed(&update, screen)?;
                    }
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Complete { source, prefix, result }))) => match result {
                        Ok((items, truncated)) => {
                            if let Some(picker) = screen.picker.as_mut()
                                && picker.source.as_deref() == Some(&source) && picker.query == prefix {
                                picker.set_items(items, truncated);
                            }
                        }
                        Err(error) => screen.notice = Some(error),
                    },
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Notice(notice)))) => screen.notice = Some(notice),
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Uploaded { generation: reply_generation, result }))) => {
                        uploads = uploads.saturating_sub(1);
                        if generation == reply_generation {
                            match result { Ok(blob) => { pending.push(blob); screen.notice = Some("Image attached; Enter sends the prompt".into()); }, Err(error) => screen.notice = Some(error) }
                        }
                    }
                    Some(Update::View(crate::Presentation::Reply(SessionReply::Sent { draft, result }))) => match result {
                        Ok(()) => screen.notice = None,
                        Err(error) => {
                            if let Some((text, attachments)) = draft {
                                if screen.editor.text().is_empty() { screen.editor.set_text(&text); }
                                else if !text.is_empty() { screen.editor.set_text(format!("{text}\n{}", screen.editor.text())); }
                                pending.extend(attachments);
                            }
                            screen.notice = Some(error);
                        }
                    },
                    None => return Ok(()),
                }
                continue;
            }
            _ = animation.tick(), if retained.has_turn() => {
                frame = frame.wrapping_add(1); continue;
            }
            event = events.recv() => match event { Some(event) => event?, None => return Ok(()) },
        };
        let view = retained.interaction();
        let out = match event {
            Event::Resize(width, height) => { screen.width = width; screen.height = height; retained.local(screen); output.invalidate(); continue; }
            Event::Paste(text) => {
                if screen.dialogs.paste(&text) {} else if crate::panel_of(&view).is_some() {
                    for character in text.chars() { screen.panel_key(&view, &crate::Key::Char(character)); }
                } else { paste(screen, &text, &commands); }
                retained.local(screen); continue;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == event::KeyCode::Char('v') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    if key.modifiers.contains(event::KeyModifiers::ALT) {
                        generation = generation.wrapping_add(1); pending.clear(); screen.notice = Some("Clipboard attachments discarded".into()); continue;
                    }
                    match clipboard.read() {
                        Ok(crate::clipboard::Contents::Text(text)) => {
                            if screen.dialogs.paste(&text) {} else if crate::panel_of(&view).is_some() { for character in text.chars() { screen.panel_key(&view, &crate::Key::Char(character)); } }
                            else { paste(screen, &text, &commands); }
                        }
                        Ok(crate::clipboard::Contents::Image { width, height, rgba }) => {
                            if screen.dialogs.focused() || crate::panel_of(&view).is_some() { screen.notice = Some("Close the input form to attach an image to the prompt".into()); continue; }
                            if pending.len() + uploads >= 16 { screen.notice = Some("Send or discard staged attachments before adding more".into()); continue; }
                            match crate::clipboard::png(width, height, rgba) {
                                Ok(bytes) => { if enqueue(&commands, Request::Upload { generation, bytes, media: "image/png".into() }, screen) { uploads += 1; } }
                                Err(error) => screen.notice = Some(error),
                            }
                        }
                        Err(error) => screen.notice = Some(error),
                    }
                    retained.local(screen); continue;
                }
                if key.code == event::KeyCode::Enter && !screen.dialogs.focused() && !key.modifiers.contains(event::KeyModifiers::SHIFT) && uploads > 0 {
                    screen.notice = Some("Wait for the attachment upload before sending".into()); continue;
                }
                if key.code == event::KeyCode::Enter && !screen.dialogs.focused() && screen.editor.text().is_empty() && !pending.is_empty() && crate::panel_of(&view).is_none() && !key.modifiers.contains(event::KeyModifiers::SHIFT) {
                    if key.modifiers.contains(event::KeyModifiers::ALT) { KeyOut::Intent(misa_kit::intent::Intent::Interrupt { text: String::new(), attachments: vec![] }) }
                    else { KeyOut::Intent(misa_kit::intent::Intent::Prompt { text: String::new(), attachments: vec![] }) }
                } else {
                let Some(key) = crate::translate(key.code, key.modifiers) else { continue; };
                screen.dialogs.key(&key)
                    .or_else(|| retained.selection_key(screen, &key))
                    .or_else(|| screen.panel_key(&view, &key))
                    .unwrap_or_else(|| screen.key(key))
                }
            }
            _ => continue,
        };
        match out {
            KeyOut::DaemonInvoke{daemon,scope,command,input}=>{enqueue(&commands,Request::DaemonInvoke{daemon,scope,command,input},screen);},
            KeyOut::Invoke { command, input } => { enqueue(&commands, Request::Invoke { command, input }, screen); },
            KeyOut::Local => {},
            KeyOut::Quit => return Ok(()),
            KeyOut::Intent(mut intent) => {
                if matches!(&intent,misa_kit::intent::Intent::Command{name,..} if name=="actions") {
                    let actions=offered_actions(&view,&contributions);
                    let mut picker=crate::Picker::new("Document actions",crate::Accept::Run);
                    picker.set_items(actions.iter().map(|(id,(_,label,origin))|misa_proto::view::Choice{value:format!("/action {id}"),label:label.clone(),detail:Some(origin.clone())}).collect(),false);
                    screen.editor.set_text(":");screen.picker=Some(picker);continue;
                }
                if let misa_kit::intent::Intent::Command{name,args}=&intent && name=="action" {
                    let actions=offered_actions(&view,&contributions);
                    let id=args.get("action").and_then(misa_value::Value::as_str).unwrap_or("");
                    let Some((node,_,_))=actions.get(id) else {screen.notice=Some("Action is no longer offered by a visible document".into());continue;};
                    intent=misa_kit::intent::Intent::Action{node:node.clone(),action:id.into(),args:misa_value::Value::Null,fields:vec![]};
                }
                if matches!(&intent, misa_kit::intent::Intent::Command { name, .. } if name == "operations") { screen.dialogs.open(); enqueue(&commands, Request::RefreshRequests, screen); continue; }
                let draft = match &mut intent {
                    misa_kit::intent::Intent::Prompt { text, attachments } | misa_kit::intent::Intent::Interrupt { text, attachments } => { attachments.extend(pending.iter().cloned()); Some(text.clone()) }, _ => None,
                };
                if enqueue(&commands, Request::Intent(intent), screen) { if draft.is_some() { pending.clear(); } }
                else if let Some(text) = draft { screen.editor.set_text(text); }
            }
            KeyOut::Complete { source, prefix } => { enqueue(&commands, Request::Complete { source, prefix }, screen); }
            KeyOut::Save(request) => match crate::save::target(&view, &request) {
                Ok(node) => { enqueue(&commands, Request::Save { node: node.into(), destination: request.destination }, screen); }
                Err(error) => screen.notice = Some(error),
            },
            KeyOut::Copy(text) => {
                use base64::Engine as _;
                let encoded = base64::engine::general_purpose::STANDARD.encode(text);
                write!(writer, "\x1b]52;c;{encoded}\x07").map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
            }
        }
        retained.local(screen);
    }
}

// The session is borrowed by this future, not by the keyboard branch. Every
// queue is bounded, and dropping the drive future cancels outstanding UI waits.
enum Update {
    View(crate::Presentation),
}
fn paste(screen: &mut Screen, text: &str, commands: &mpsc::Sender<Request>) {
    if let KeyOut::Complete { source, prefix } = screen.paste(text) {
        enqueue(commands, Request::Complete { source, prefix }, screen);
    }
}
fn enqueue(sender: &mpsc::Sender<Request>, request: Request, screen: &mut Screen) -> bool {
    if sender.try_send(request).is_err() {
        screen.notice = Some("The session request queue is full; try again shortly".into());
        false
    } else {
        true
    }
}
async fn requests_loop(session: &mut dyn Session, mut requests: mpsc::Receiver<Request>, updates: mpsc::Sender<Update>) -> Result<(), String> {
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
        if updates.send(update).await.is_err() { return Ok(()); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::Node;
use misa_kit::intent::Intent;
    use misa_proto::view::Choice;
    struct Idle { first: bool }
    #[async_trait::async_trait]
    impl Session for Idle {
        async fn next(&mut self) -> Result<Option<Node>, String> {
            if self.first { self.first = false; return Ok(Some(Node::section("session").id("session"))); }
            std::future::pending().await
        }
        async fn send(&mut self, _: Intent) -> Result<(), String> { Ok(()) }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> { Ok((vec![], false)) }
    }
    #[tokio::test]
    async fn an_idle_session_still_accepts_paste_resize_and_quit() {
        let mut session = Idle { first: true };
        let mut screen = Screen::new(80, 24);
        let (sender, receiver) = mpsc::channel(64);
        sender.try_send(Ok(Event::Paste("two\nlines".into()))).unwrap();
        sender.try_send(Ok(Event::Resize(100, 30))).unwrap();
        sender.try_send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)))).unwrap();
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), drive(&mut session, &mut screen, receiver, &mut output))
            .await.expect("idle session blocked local input").unwrap();
        assert_eq!(screen.editor.text(), "two\nlines");
        assert_eq!((screen.width, screen.height), (100, 30));
    }

    struct Waiting { started: Arc<tokio::sync::Notify> }
    #[async_trait::async_trait]
    impl Session for Waiting {
        async fn next(&mut self) -> Result<Option<Node>, String> { std::future::pending().await }
        async fn send(&mut self, _: Intent) -> Result<(), String> { std::future::pending().await }
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
        let mut session = Waiting { started: started.clone() };
        let mut screen = Screen::new(80, 24);
        screen.commands = vec![misa_kit::intent::Command::new("model", "Model", "choose")
            .arg(misa_proto::preparation::Arg::new("model", "Model").required().from("models"))];
        screen.sources = vec![misa_kit::intent::Source::resident("models", "Models")];
        screen.editor.set_text("/model");
        let (sender, receiver) = mpsc::channel(64);
        sender.try_send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Tab, event::KeyModifiers::NONE)))).unwrap();
        let input = async move {
            started.notified().await;
            sender.try_send(Ok(Event::Paste("still editing".into()))).unwrap();
            sender.try_send(Ok(Event::Resize(100, 30))).unwrap();
            sender.try_send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)))).unwrap();
        };
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), async {
            let (result, ()) = tokio::join!(drive(&mut session, &mut screen, receiver, &mut output), input);
            result.unwrap();
        }).await.expect("completion blocked local input");
        assert!(screen.editor.text().contains("still editing"));
        assert_eq!((screen.width, screen.height), (100, 30));
    }

    #[tokio::test]
    async fn startup_does_not_wait_for_a_snapshot_before_quit() {
        let mut session = Waiting { started: Default::default() };
        let mut screen = Screen::new(80, 24);
        let (sender, receiver) = mpsc::channel(64);
        sender.try_send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)))).unwrap();
        tokio::time::timeout(Duration::from_secs(1), drive(&mut session, &mut screen, receiver, &mut Vec::new()))
            .await.expect("startup blocked quit").unwrap();
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use misa_proto::Node;
use misa_kit::intent::Intent;
    use misa_proto::view::{BlobRef, Choice};
    use std::sync::atomic::AtomicUsize;
    struct ImageClipboard;
    impl crate::clipboard::Source for ImageClipboard {
        fn read(&mut self) -> Result<crate::clipboard::Contents, String> { Ok(crate::clipboard::Contents::Image { width: 1, height: 1, rgba: vec![255, 0, 0, 255] }) }
    }
    struct FrameWriter { bytes: Vec<u8>, frame: Arc<AtomicUsize>, changed: tokio::sync::watch::Sender<usize> }
    impl Write for FrameWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.bytes.extend_from_slice(bytes); Ok(bytes.len()) }
        fn flush(&mut self) -> std::io::Result<()> {
            let frame = self.frame.fetch_add(1, Ordering::SeqCst) + 1;
            self.changed.send_replace(frame); Ok(())
        }
    }
    struct UploadSession { fail: bool, uploads: usize, sent: Vec<Intent>, frame: Arc<AtomicUsize>, receipts: mpsc::Sender<usize> }
    #[async_trait::async_trait]
    impl Session for UploadSession {
        async fn next(&mut self) -> Result<Option<Node>, String> { std::future::pending().await }
        async fn send(&mut self, intent: Intent) -> Result<(), String> {
            self.sent.push(intent);
            self.receipts.send(self.frame.load(Ordering::SeqCst)).await.unwrap();
            if self.fail { self.fail = false; Err("retry send".into()) } else { Ok(()) }
        }
        async fn upload(&mut self, bytes: Vec<u8>, media: &str) -> Result<BlobRef, String> {
            assert_eq!(media, "image/png");
            assert_eq!(image::load_from_memory(&bytes).unwrap().into_rgba8().into_raw(), vec![255, 0, 0, 255]);
            self.uploads += 1;
            self.receipts.send(self.frame.load(Ordering::SeqCst)).await.unwrap();
            Ok(BlobRef { hash: format!("blob-{}", self.uploads), len: bytes.len() as u64, media: Some(media.into()) })
        }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> { Ok((vec![], false)) }
    }
    fn key(code: event::KeyCode, modifiers: event::KeyModifiers) -> Result<Event, String> { Ok(Event::Key(event::KeyEvent::new(code, modifiers))) }
    async fn acknowledged(receipts: &mut mpsc::Receiver<usize>, frames: &mut tokio::sync::watch::Receiver<usize>) {
        let before = receipts.recv().await.unwrap();
        while *frames.borrow_and_update() <= before { frames.changed().await.unwrap(); }
    }
    #[tokio::test]
    async fn staged_images_retry_exactly_and_discard_without_a_prompt() {
        let frame = Arc::new(AtomicUsize::new(0));
        let (changed, mut frames) = tokio::sync::watch::channel(0);
        let (receipts, mut received) = mpsc::channel(8);
        let mut session = UploadSession { fail: true, uploads: 0, sent: vec![], frame: frame.clone(), receipts };
        let mut writer = FrameWriter { bytes: vec![], frame, changed };
        let mut screen = Screen::new(80, 24); screen.editor.set_text("a picture");
        let (sender, receiver) = mpsc::channel(64);
        let script = async move {
            sender.try_send(key(event::KeyCode::Char('v'), event::KeyModifiers::CONTROL)).unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender.try_send(key(event::KeyCode::Enter, event::KeyModifiers::NONE)).unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender.try_send(key(event::KeyCode::Enter, event::KeyModifiers::NONE)).unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender.try_send(key(event::KeyCode::Char('v'), event::KeyModifiers::CONTROL)).unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender.try_send(key(event::KeyCode::Char('v'), event::KeyModifiers::CONTROL | event::KeyModifiers::ALT)).unwrap();
            sender.try_send(key(event::KeyCode::Enter, event::KeyModifiers::NONE)).unwrap();
            sender.try_send(key(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)).unwrap();
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut clipboard = ImageClipboard; let (result, ()) = tokio::join!(drive_with_clipboard(&mut session, &mut screen, receiver, &mut writer, &mut clipboard), script); result.unwrap();
        }).await.expect("clipboard requests stalled");
        assert_eq!(session.uploads, 2); assert_eq!(session.sent.len(), 2); assert_eq!(session.sent[0], session.sent[1]);
        assert!(matches!(&session.sent[1], Intent::Prompt { text, attachments } if text == "a picture" && attachments.len() == 1 && attachments[0].hash == "blob-1"));
        assert!(screen.editor.text().is_empty());
        assert!(String::from_utf8(writer.bytes).unwrap().contains("1 clipboard attachments"));
    }
    #[tokio::test]
    async fn image_only_alt_enter_carries_the_staged_ref() {
        let frame = Arc::new(AtomicUsize::new(0));
        let (changed, mut frames) = tokio::sync::watch::channel(0);
        let (receipts, mut received) = mpsc::channel(8);
        let mut session = UploadSession { fail: false, uploads: 0, sent: vec![], frame: frame.clone(), receipts };
        let mut writer = FrameWriter { bytes: vec![], frame, changed };
        let mut screen = Screen::new(80, 24);
        let (sender, receiver) = mpsc::channel(64);
        let script = async move {
            sender.try_send(key(event::KeyCode::Char('v'), event::KeyModifiers::CONTROL)).unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender.try_send(key(event::KeyCode::Enter, event::KeyModifiers::ALT)).unwrap();
            acknowledged(&mut received, &mut frames).await;
            sender.try_send(key(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)).unwrap();
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut clipboard = ImageClipboard; let (result, ()) = tokio::join!(drive_with_clipboard(&mut session, &mut screen, receiver, &mut writer, &mut clipboard), script); result.unwrap();
        }).await.unwrap();
        assert!(matches!(&session.sent[..], [Intent::Interrupt { text, attachments }] if text.is_empty() && attachments.len() == 1));
    }
}

#[cfg(test)]
mod save_liveness_test {
    use super::*;
    use misa_proto::Node;
use misa_kit::intent::Intent;
    use misa_proto::view::{Action, ActionOn, Choice};
    struct Saving { first: bool, started: Arc<tokio::sync::Notify> }
    #[async_trait::async_trait]
    impl Session for Saving {
        async fn next(&mut self) -> Result<Option<Node>, String> {
            if self.first { self.first = false; return Ok(Some(Node::section("session").id("session").child(Node::section("attachment").id("file").action(Action { id: "attachment.save".into(), on: ActionOn::Click, label: None, args: misa_value::Value::Null })))); }
            std::future::pending().await
        }
        async fn send(&mut self, _: Intent) -> Result<(), String> { Ok(()) }
        async fn save_attachment(&mut self, node: &str, _: &str) -> Result<(), String> { assert_eq!(node, "file"); self.started.notify_one(); std::future::pending().await }
        async fn complete(&mut self, _: &str, _: &str) -> Result<(Vec<Choice>, bool), String> { Ok((vec![], false)) }
    }
    struct Observed { bytes: Vec<u8>, ready: Arc<tokio::sync::Notify> }
    impl Write for Observed {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.bytes.extend_from_slice(bytes); Ok(bytes.len()) }
        fn flush(&mut self) -> std::io::Result<()> {
            if String::from_utf8_lossy(&self.bytes).contains("1 attachments") { self.ready.notify_one(); }
            Ok(())
        }
    }
    #[tokio::test]
    async fn waiting_for_a_save_keeps_edit_resize_and_quit_live() {
        let ready = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(tokio::sync::Notify::new());
        let mut session = Saving { first: true, started: started.clone() };
        let mut screen = Screen::new(80, 24); screen.editor.set_text("/save /tmp/unused-save-test");
        let mut writer = Observed { bytes: vec![], ready: ready.clone() };
        let (sender, receiver) = mpsc::channel(64);
        let input = async move {
            ready.notified().await;
            sender.try_send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Enter, event::KeyModifiers::NONE)))).unwrap();
            started.notified().await;
            sender.try_send(Ok(Event::Paste("still editing".into()))).unwrap();
            sender.try_send(Ok(Event::Resize(90, 30))).unwrap();
            sender.try_send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)))).unwrap();
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            let (result, ()) = tokio::join!(drive(&mut session, &mut screen, receiver, &mut writer), input); result.unwrap();
        }).await.expect("save wait blocked keyboard");
        assert_eq!(screen.editor.text(), "still editing"); assert_eq!((screen.width, screen.height), (90, 30));
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    struct SurfaceSession {updates:mpsc::Receiver<crate::Presentation>,received:mpsc::Sender<usize>,frames:Arc<std::sync::atomic::AtomicUsize>,sent:Option<mpsc::Sender<misa_kit::intent::Intent>>}
    #[async_trait::async_trait]
    impl Session for SurfaceSession {
        async fn complete(&mut self,_source:&str,_prefix:&str)->Result<(Vec<misa_proto::view::Choice>,bool),String>{Ok((vec![],false))}
        async fn next_presentation(&mut self)->Result<Option<crate::Presentation>,String>{
            let next=self.updates.recv().await;
            if next.is_some(){let _=self.received.send(self.frames.load(Ordering::SeqCst)).await;}
            Ok(next)
        }
        async fn next(&mut self)->Result<Option<misa_proto::Node>,String>{std::future::pending().await}
        async fn send(&mut self,intent:misa_kit::intent::Intent)->Result<(),String>{if let Some(sent)=&self.sent {sent.send(intent).await.unwrap();}Ok(())}
    }
    struct Writer{frames:Arc<std::sync::atomic::AtomicUsize>,changed:tokio::sync::watch::Sender<usize>}
    impl Write for Writer{
        fn write(&mut self,bytes:&[u8])->std::io::Result<usize>{Ok(bytes.len())}
        fn flush(&mut self)->std::io::Result<()>{self.changed.send_replace(self.frames.fetch_add(1,Ordering::SeqCst)+1);Ok(())}
    }
    async fn painted(frames:&mut tokio::sync::watch::Receiver<usize>,after:usize){while *frames.borrow_and_update()<=after{frames.changed().await.unwrap();}}
    #[tokio::test]
    async fn scope_switch_preserves_composer_and_hidden_private_draft(){
        let frame=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (changed,mut frames)=tokio::sync::watch::channel(0);
        let (updates,receive_updates)=mpsc::channel(8);
        let (received,mut receipts)=mpsc::channel(8);
        let mut session=SurfaceSession{updates:receive_updates,received,frames:frame.clone(),sent:None};
        let mut writer=Writer{frames:frame.clone(),changed};
        let mut screen=Screen::new(80,24);
        screen.prefs.drafts.insert("A".into(),"draft A".into());
        screen.dialogs.update("secret".into(),1,Some(misa_client::request::Model{form:None,id:"secret".into(),generation:1,title:"Credential".into(),body:misa_proto::Node::section("request").id("request"),input:Some(misa_client::request::Input{id:"value".into(),label:"Key".into(),secret:true}),actions:vec![]}));
        screen.dialogs.open();screen.dialogs.key(&crate::Key::Char('s'));screen.dialogs.key(&crate::Key::Escape);
        let (keys,events)=mpsc::channel(8);
        let script=async move {
            for scope in ["A","B"] {
                updates.send(crate::Presentation::Activate(scope.into())).await.unwrap();
                painted(&mut frames,receipts.recv().await.unwrap()).await;
            }
            let before=frame.load(Ordering::SeqCst);
            keys.send(Ok(Event::Paste("draft B".into()))).await.unwrap();
            painted(&mut frames,before).await;
            updates.send(crate::Presentation::Activate("A".into())).await.unwrap();
            painted(&mut frames,receipts.recv().await.unwrap()).await;
            keys.send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'),event::KeyModifiers::CONTROL)))).await.unwrap();
        };
        tokio::time::timeout(Duration::from_secs(3),async {let (result,())=tokio::join!(drive(&mut session,&mut screen,events,&mut writer),script);result.unwrap();}).await.unwrap();
        assert_eq!(screen.editor.text(),"draft A");
        screen.dialogs.open();screen.dialogs.key(&crate::Key::Char('t'));
        let text=misa_render::to_plain(&screen.dialogs.lines(&screen.theme,80));
        assert!(text.contains("••"),"hidden secret draft was lost: {text}");
    }
    #[test]
    fn optional_documents_contribute_portable_actions_without_merging_node_spaces(){
        let screen=Screen::new(80,24);
        let root=misa_proto::Node::section("conversation").id("same");
        let mut pet=misa_proto::Node::section("pet").id("same");
        pet.actions.push(misa_proto::view::Action{id:"pet.feed".into(),label:Some("Feed pet".into()),on:Default::default(),args:misa_value::Value::Null});
        let mut documents=std::collections::BTreeMap::from([("plugin.pet.companion".into(),crate::retained::Retained::new(pet,&screen))]);
        let actions=offered_actions(&root,&documents);
        assert_eq!(actions["pet.feed"],("same".into(),"Feed pet".into(),"plugin.pet.companion".into()));
        documents.clear();
        assert!(!offered_actions(&root,&documents).contains_key("pet.feed"),"hidden documents cannot leave active action candidates");
    }
    #[tokio::test]
    async fn optional_document_action_chooser_invokes_its_offered_binding(){
        let frame=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (changed,mut frames)=tokio::sync::watch::channel(0);
        let (updates,receive_updates)=mpsc::channel(8);
        let (received,mut receipts)=mpsc::channel(8);
        let (sent,mut intents)=mpsc::channel(8);
        let mut session=SurfaceSession{updates:receive_updates,received,frames:frame.clone(),sent:Some(sent)};
        let mut writer=Writer{frames:frame.clone(),changed};
        let mut screen=Screen::new(80,24);
        screen.commands=vec![misa_kit::intent::Command::new("actions","Actions","Actions"),misa_kit::intent::Command::new("action","Action","Action").arg(misa_proto::preparation::Arg::new("action","Action").required())];
        screen.editor.set_text("/actions");
        let mut pet=misa_proto::Node::section("pet").id("pet");
        pet.actions.push(misa_proto::view::Action{id:"pet.feed".into(),label:Some("Feed pet".into()),on:Default::default(),args:misa_value::Value::Null});
        let (keys,events)=mpsc::channel(8);
        let script=async move {
            updates.send(crate::Presentation::Documents(vec![("plugin.pet.companion".into(),misa_client::document::Update::Reset(misa_proto::observation::Document{version:misa_proto::sync::Version{epoch:"pet".into(),rev:0},tree:pet,streams:vec![]}))])).await.unwrap();
            painted(&mut frames,receipts.recv().await.unwrap()).await;
            let before=frame.load(Ordering::SeqCst);
            keys.send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Enter,event::KeyModifiers::NONE)))).await.unwrap();
            painted(&mut frames,before).await;
            keys.send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Enter,event::KeyModifiers::NONE)))).await.unwrap();
            assert!(matches!(intents.recv().await.unwrap(),misa_kit::intent::Intent::Action{node,action,..} if node=="pet" && action=="pet.feed"));
            keys.send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'),event::KeyModifiers::CONTROL)))).await.unwrap();
        };
        tokio::time::timeout(Duration::from_secs(3),async {let (result,())=tokio::join!(drive(&mut session,&mut screen,events,&mut writer),script);result.unwrap();}).await.expect("optional document action was not usable");
    }
}
