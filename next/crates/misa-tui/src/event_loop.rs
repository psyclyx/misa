//! Keyboard and local animation remain live while the session has nothing to send.
use std::io::Write;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc;
use crate::{KeyOut, Screen, Session};
use crate::{SessionRequest as Request, SessionReply};

struct Terminal;
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
    if let Some(info) = session.info() { screen.declare(&info); }
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
    let mut pending = Vec::<misa_proto::view::BlobRef>::new();
    let mut uploads = 0usize;
    let mut generation = 0u64;
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
        let rendered = retained.frame(screen, staging.as_deref());
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
                    Some(Update::View(crate::Presentation::Snapshot(next))) => retained = crate::retained::Retained::new(next, screen),
                    Some(Update::View(crate::Presentation::Message(message))) => retained.receive(&message, screen)?,
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
                if crate::panel_of(&view).is_some() {
                    for character in text.chars() { screen.panel_key(&view, &crate::Key::Char(character)); }
                } else { screen.editor.insert(&text); }
                retained.local(screen); continue;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == event::KeyCode::Char('v') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    if key.modifiers.contains(event::KeyModifiers::ALT) {
                        generation = generation.wrapping_add(1); pending.clear(); screen.notice = Some("Clipboard attachments discarded".into()); continue;
                    }
                    match clipboard.read() {
                        Ok(crate::clipboard::Contents::Text(text)) => {
                            if crate::panel_of(&view).is_some() { for character in text.chars() { screen.panel_key(&view, &crate::Key::Char(character)); } }
                            else { screen.editor.insert(&text); }
                        }
                        Ok(crate::clipboard::Contents::Image { width, height, rgba }) => {
                            if crate::panel_of(&view).is_some() { screen.notice = Some("Close the panel to attach an image to the prompt".into()); continue; }
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
                if key.code == event::KeyCode::Enter && !key.modifiers.contains(event::KeyModifiers::SHIFT) && uploads > 0 {
                    screen.notice = Some("Wait for the attachment upload before sending".into()); continue;
                }
                if key.code == event::KeyCode::Enter && screen.editor.text().is_empty() && !pending.is_empty() && crate::panel_of(&view).is_none() && !key.modifiers.contains(event::KeyModifiers::SHIFT) {
                    if key.modifiers.contains(event::KeyModifiers::ALT) { KeyOut::Intent(misa_proto::Intent::Interrupt { text: String::new(), attachments: vec![] }) }
                    else { KeyOut::Intent(misa_proto::Intent::Prompt { text: String::new(), attachments: vec![] }) }
                } else {
                let Some(key) = crate::translate(key.code, key.modifiers) else { continue; };
                retained.selection_key(screen, &key)
                    .or_else(|| screen.panel_key(&view, &key))
                    .unwrap_or_else(|| screen.key(key))
                }
            }
            _ => continue,
        };
        match out {
            KeyOut::Local => {},
            KeyOut::Quit => return Ok(()),
            KeyOut::Intent(mut intent) => {
                let draft = match &mut intent {
                    misa_proto::Intent::Prompt { text, attachments } | misa_proto::Intent::Interrupt { text, attachments } => { attachments.extend(pending.iter().cloned()); Some(text.clone()) }, _ => None,
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
    use misa_proto::{Node, Intent, SessionInfo};
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
        fn info(&self) -> Option<SessionInfo> { None }
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
        fn info(&self) -> Option<SessionInfo> { None }
    }
    #[tokio::test]
    async fn a_slow_completion_keeps_paste_resize_and_quit_live() {
        let started = Arc::new(tokio::sync::Notify::new());
        let mut session = Waiting { started: started.clone() };
        let mut screen = Screen::new(80, 24);
        screen.commands = vec![misa_proto::wire::Command::new("model", "Model", "choose")
            .arg(misa_proto::wire::Arg::new("model", "Model").required().from("models"))];
        screen.sources = vec![misa_proto::wire::Source::resident("models", "Models")];
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
    use misa_proto::{Node, Intent, SessionInfo};
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
        fn info(&self) -> Option<SessionInfo> { None }
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
    use misa_proto::{Intent, Node, SessionInfo};
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
        fn info(&self) -> Option<SessionInfo> { None }
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
