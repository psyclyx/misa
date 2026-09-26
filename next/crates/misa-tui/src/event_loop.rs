//! Keyboard and local animation remain live while the session has nothing to send.
#[cfg(test)]
use crate::KeyOut;
use crate::{ConnectedScreen, Session};
mod input;
mod paint;
mod scopes;
mod updates;
use crate::SessionRequest as Request;
#[cfg(test)]
use crossterm::event;
use crossterm::event::Event;
#[cfg(test)]
use input::offered_actions;
use input::{InputOutcome, handle_input};
use paint::ConnectedPainter;
use scopes::Scopes;
use std::io::Write;
#[cfg(test)]
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use updates::handle_update;

pub async fn run(session: &mut dyn Session) -> Result<(), String> {
    let mut screen = ConnectedScreen::durable();
    screen.declare(&session.catalog());
    screen.ui.location = session.location();
    if let Ok((width, height)) = crossterm::terminal::size() {
        screen.ui.width = width;
        screen.ui.height = height;
    }
    // Kitty support is advertised, never probed synchronously. A terminal that
    // reports its pixel geometry also tells us how big a cell is; otherwise the
    // default ratio is used.
    screen.ui.graphics = misa_terminal_ui::graphics::Kitty::detect();
    if let Ok(size) = crossterm::terminal::window_size()
        && let Some(cell) = misa_terminal_ui::graphics::CellSize::from_window(
            screen.ui.width,
            screen.ui.height,
            size.width,
            size.height,
        )
    {
        screen.ui.graphics.set_cell(cell);
    }
    let _terminal = misa_terminal_runtime::Terminal::enter()?;
    let (_reader, receiver) = misa_terminal_runtime::events();
    let result = drive(session, &mut screen, receiver, &mut std::io::stdout()).await;
    screen.ui.save();
    result
}

async fn drive(
    session: &mut dyn Session,
    screen: &mut ConnectedScreen,
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
    screen: &mut ConnectedScreen,
    mut events: mpsc::Receiver<Result<Event, String>>,
    writer: &mut impl Write,
    clipboard: &mut dyn crate::clipboard::Source,
) -> Result<(), String> {
    let mut scopes = Scopes::new(screen);
    let mut painter = ConnectedPainter::new();
    // The owner serializes work; the UI must not turn that serialization into dropped input.
    let (commands, requests) = mpsc::unbounded_channel();
    // A presentation is a fact the UI has to observe eventually. Letting the update pipe
    // backpressure the request owner is what turns a burst of startup notices into a full
    // input queue.
    let (updates, mut incoming) = mpsc::unbounded_channel();
    let driver = requests_loop(session, requests, updates);
    tokio::pin!(driver);
    let mut animation = tokio::time::interval(Duration::from_millis(90));
    animation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        // Keep discovery and paint ahead of select, including request send timing.
        for blob in painter.downloads(&scopes, screen) {
            enqueue(&commands, Request::Download { reference: blob }, screen);
        }
        painter.paint(&mut scopes, screen, writer)?;
        let event = tokio::select! {
            result = &mut driver => return result,
            update = incoming.recv() => {
                match handle_update(update, &mut scopes, screen, &commands)? {
                    Some(change) => painter.changed(change),
                    None => return Ok(()),
                }
                continue;
            }
            _ = animation.tick(), if scopes.active.retained.has_turn() => {
                painter.tick(); continue;
            }
            event = events.recv() => match event { Some(event) => event?, None => return Ok(()) },
        };
        if handle_input(
            event,
            &mut scopes,
            screen,
            &commands,
            writer,
            clipboard,
            &mut painter,
        )? == InputOutcome::Quit
        {
            return Ok(());
        }
    }
}

// The session is borrowed by this future, not by the keyboard branch. Dropping the drive
// future cancels outstanding UI waits.
enum Update {
    View(crate::Presentation),
}
fn enqueue(
    sender: &mpsc::UnboundedSender<Request>,
    request: Request,
    screen: &mut ConnectedScreen,
) -> bool {
    if sender.send(request).is_err() {
        screen.ui.notice = Some("The session request owner is closed".into());
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
        let mut screen = ConnectedScreen::new(80, 24);
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
        assert_eq!(screen.ui.composer.text(), "two\nlines");
        assert_eq!((screen.ui.width, screen.ui.height), (100, 30));
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
        let mut screen = ConnectedScreen::new(80, 24);
        screen.declare(&misa_tui_app::Catalog {
            commands: vec![
                misa_kit::intent::Command::new("model", "Model", "choose").arg(
                    misa_proto::preparation::Arg::new("model", "Model")
                        .required()
                        .from("models"),
                ),
            ],
            sources: vec![misa_kit::intent::Source::resident("models", "Models")],
        });
        screen.ui.composer.set_text("/model");
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
        assert!(screen.ui.composer.text().contains("still editing"));
        assert_eq!((screen.ui.width, screen.ui.height), (100, 30));
    }

    #[tokio::test]
    async fn startup_does_not_wait_for_a_snapshot_before_quit() {
        let mut session = Waiting {
            started: Default::default(),
        };
        let mut screen = ConnectedScreen::new(80, 24);
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
        let mut screen = ConnectedScreen::new(80, 24);
        screen.ui.composer.set_text("a picture");
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
        assert!(screen.ui.composer.text().is_empty());
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
        let mut screen = ConnectedScreen::new(80, 24);
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
        let mut screen = ConnectedScreen::new(80, 24);
        screen.declare(&session.catalog());
        screen.ui.composer.set_text("/save /tmp/unused-save-test");
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
        assert_eq!(screen.ui.composer.text(), "still editing");
        assert_eq!((screen.ui.width, screen.ui.height), (90, 30));
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
        let mut screen = ConnectedScreen::new(80, 24);
        screen.ui.set_draft_for("A".into(), "draft A".into());
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
            .key(&crate::Key::Char('s'), &screen.ui.dialog_settings().clone());
        screen
            .dialogs
            .key(&crate::Key::Escape, &screen.ui.dialog_settings().clone());
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
        assert_eq!(screen.ui.composer.text(), "draft A");
        screen.dialogs.open();
        screen
            .dialogs
            .key(&crate::Key::Char('t'), &screen.ui.dialog_settings().clone());
        let text = misa_lines::to_plain(&screen.dialogs.lines(
            &screen.ui.theme,
            80,
            screen.ui.dialog_settings(),
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
        let mut screen = ConnectedScreen::new(80, 24);
        let original = misa_tui_app::Catalog {
            commands: vec![
                Command::new("model", "Model", "choose")
                    .arg(Arg::new("model", "Model").required().from("models")),
            ],
            sources: vec![Source::resident("models", "Models")],
        };
        screen.declare(&original);
        screen.ui.composer.set_text("/model");
        assert!(matches!(
            screen.ui.key(crate::Key::Submit),
            KeyOut::Complete { .. }
        ));
        screen.ui.key(crate::Key::Char('f'));
        screen.ui.set_draft_for("A".into(), "/model f".into());
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
        assert_eq!(screen.ui.composer.text(), "/model f");
        assert_eq!(screen.ui.composer.picker().unwrap().query, "f");
        assert!(
            screen
                .ui
                .command_candidates()
                .iter()
                .any(|c| c.value == "/fresh")
        );
        screen.ui.key(crate::Key::Escape);
        assert_eq!(
            screen.ui.key(crate::Key::Action(crate::Action::OpenModel)),
            KeyOut::Local
        );
        assert_eq!(
            screen
                .ui
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
        let screen = ConnectedScreen::new(80, 24);
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
            crate::retained::Retained::new(pet, &screen.ui),
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
        let mut screen = ConnectedScreen::new(80, 24);
        screen.declare(&misa_tui_app::Catalog {
            commands: vec![
                misa_kit::intent::Command::new("actions", "Actions", "Actions"),
                misa_kit::intent::Command::new("action", "Action", "Action")
                    .arg(misa_proto::preparation::Arg::new("action", "Action").required()),
            ],
            sources: vec![],
        });
        screen.ui.composer.set_text("/actions");
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
