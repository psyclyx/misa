//! Keyboard and local animation remain live while the session has nothing to send.
use std::io::Write;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc;
use crate::{KeyOut, Screen, Session};

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
    let (sender, receiver) = mpsc::unbounded_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let reading = stop.clone();
    let thread = std::thread::spawn(move || {
        while !reading.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(50)) {
                Ok(false) => continue,
                Ok(true) => if sender.send(event::read().map_err(|error| error.to_string())).is_err() { break; },
                Err(error) => { let _ = sender.send(Err(error.to_string())); break; }
            }
        }
    });
    let _reader = Reader { stop, thread: Some(thread) };
    let result = drive(session, &mut screen, receiver, &mut std::io::stdout()).await;
    screen.save();
    result
}

async fn drive(session: &mut dyn Session, screen: &mut Screen,
    mut events: mpsc::UnboundedReceiver<Result<Event, String>>, writer: &mut impl Write) -> Result<(), String> {
    let mut view = misa_proto::Node::section("session").id("session");
    let (commands, requests) = mpsc::channel(16);
    let (updates, mut incoming) = mpsc::channel(1);
    let driver = requests_loop(session, requests, updates);
    tokio::pin!(driver);
    let mut output = crate::output::Output::default();
    let mut animation = tokio::time::interval(Duration::from_millis(90));
    animation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame = 0usize;
    loop {
        let mut lines = crate::draw(screen, &view);
        if misa_proto::view::find(&view, "turn").is_some() {
            const FRAMES: [&str; 4] = ["⠋", "⠙", "⠹", "⠸"];
            if let Some(line) = lines.iter_mut().find(|line| line.node.as_deref() == Some("turn")) {
                line.spans.insert(0, (screen.theme.role("turn"), format!("{} ", FRAMES[frame % FRAMES.len()])));
            }
        }
        output.paint(writer, &lines).map_err(|error| error.to_string())?;
        let (before, _) = screen.editor.split_at_cursor();
        let column = (2 + misa_render::width(before.rsplit('\n').next().unwrap_or("")))
            .min(screen.width.saturating_sub(1) as usize);
        write!(writer, "\x1b[{};{}H", lines.len().max(1), column + 1)
            .and_then(|_| writer.flush()).map_err(|error| error.to_string())?;
        let event = tokio::select! {
            result = &mut driver => return result,
            update = incoming.recv() => {
                match update {
                    Some(Update::View(next)) => view = next,
                    Some(Update::Complete { source, prefix, result }) => match result {
                        Ok((items, truncated)) => {
                            if let Some(picker) = screen.picker.as_mut()
                                && picker.source.as_deref() == Some(&source) && picker.query == prefix {
                                picker.set_items(items, truncated);
                            }
                        }
                        Err(error) => screen.notice = Some(error),
                    },
                    Some(Update::Notice(notice)) => screen.notice = Some(notice),
                    None => return Ok(()),
                }
                continue;
            }
            _ = animation.tick(), if misa_proto::view::find(&view, "turn").is_some() => {
                frame = frame.wrapping_add(1); continue;
            }
            event = events.recv() => match event { Some(event) => event?, None => return Ok(()) },
        };
        let out = match event {
            Event::Resize(width, height) => { screen.width = width; screen.height = height; output.invalidate(); continue; }
            Event::Paste(text) => {
                if crate::panel_of(&view).is_some() {
                    for character in text.chars() { screen.panel_key(&view, &crate::Key::Char(character)); }
                } else { screen.editor.insert(&text); }
                continue;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let Some(key) = crate::translate(key.code, key.modifiers) else { continue; };
                screen.selection_key(&view, &key)
                    .or_else(|| screen.panel_key(&view, &key))
                    .unwrap_or_else(|| screen.key(key))
            }
            _ => continue,
        };
        match out {
            KeyOut::Local => {},
            KeyOut::Quit => return Ok(()),
            KeyOut::Intent(intent) => enqueue(&commands, Request::Intent(intent), screen),
            KeyOut::Complete { source, prefix } => enqueue(&commands, Request::Complete { source, prefix }, screen),
            KeyOut::Save(request) => match crate::save::target(&view, &request) {
                Ok(node) => enqueue(&commands, Request::Save { node: node.into(), destination: request.destination }, screen),
                Err(error) => screen.notice = Some(error),
            },
            KeyOut::Copy(text) => {
                use base64::Engine as _;
                let encoded = base64::engine::general_purpose::STANDARD.encode(text);
                write!(writer, "\x1b]52;c;{encoded}\x07").map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
            }
        }
    }
}

// The session is borrowed by this future, not by the keyboard branch. Every
// queue is bounded, and dropping the drive future cancels outstanding UI waits.
enum Request {
    Intent(misa_proto::Intent),
    Complete { source: String, prefix: String },
    Save { node: String, destination: String },
}
enum Update {
    View(misa_proto::Node),
    Complete { source: String, prefix: String, result: Result<(Vec<misa_proto::view::Choice>, bool), String> },
    Notice(String),
}
fn enqueue(sender: &mpsc::Sender<Request>, request: Request, screen: &mut Screen) {
    if sender.try_send(request).is_err() {
        screen.notice = Some("The session request queue is full; try again shortly".into());
    }
}
async fn requests_loop(session: &mut dyn Session, mut requests: mpsc::Receiver<Request>, updates: mpsc::Sender<Update>) -> Result<(), String> {
    loop {
        let update = tokio::select! {
            request = requests.recv() => match request {
                None => return Ok(()),
                Some(Request::Intent(intent)) => match session.send(intent).await {
                    Ok(()) => continue, Err(error) => Update::Notice(error),
                },
                Some(Request::Complete { source, prefix }) => {
                    let result = session.complete(&source, &prefix).await;
                    Update::Complete { source, prefix, result }
                }
                Some(Request::Save { node, destination }) => Update::Notice(match session.save_attachment(&node, &destination).await {
                    Ok(()) => format!("Saved {destination}"), Err(error) => error,
                }),
            },
            incoming = session.next() => match incoming? {
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
        let (sender, receiver) = mpsc::unbounded_channel();
        sender.send(Ok(Event::Paste("two\nlines".into()))).unwrap();
        sender.send(Ok(Event::Resize(100, 30))).unwrap();
        sender.send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)))).unwrap();
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
        let (sender, receiver) = mpsc::unbounded_channel();
        sender.send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Tab, event::KeyModifiers::NONE)))).unwrap();
        let input = async move {
            started.notified().await;
            sender.send(Ok(Event::Paste("still editing".into()))).unwrap();
            sender.send(Ok(Event::Resize(100, 30))).unwrap();
            sender.send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)))).unwrap();
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
        let (sender, receiver) = mpsc::unbounded_channel();
        sender.send(Ok(Event::Key(event::KeyEvent::new(event::KeyCode::Char('q'), event::KeyModifiers::CONTROL)))).unwrap();
        tokio::time::timeout(Duration::from_secs(1), drive(&mut session, &mut screen, receiver, &mut Vec::new()))
            .await.expect("startup blocked quit").unwrap();
    }
}
