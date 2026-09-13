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
    let Some(mut view) = session.next().await? else { return Ok(()); };
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
            incoming = session.next() => {
                match incoming? { Some(next) => view = next, None => return Ok(()) }
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
            KeyOut::Intent(intent) => if let Err(error) = session.send(intent).await { screen.notice = Some(error); },
            KeyOut::Complete { source, prefix } => match session.complete(&source, &prefix).await {
                Ok((items, truncated)) => screen.candidates(&source, items, truncated),
                Err(error) => screen.notice = Some(error),
            },
            KeyOut::Save(request) => screen.notice = Some(match crate::save::target(&view, &request) {
                Ok(node) => match session.save_attachment(node, &request.destination).await {
                    Ok(()) => format!("Saved {}", request.destination), Err(error) => error,
                }, Err(error) => error,
            }),
            KeyOut::Copy(text) => {
                use base64::Engine as _;
                let encoded = base64::engine::general_purpose::STANDARD.encode(text);
                write!(writer, "\x1b]52;c;{encoded}\x07").map_err(|error| error.to_string())?;
                writer.flush().map_err(|error| error.to_string())?;
            }
        }
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
}
