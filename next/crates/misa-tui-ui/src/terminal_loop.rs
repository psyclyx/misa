//! Terminal lifecycle, event intake and frame emission shared by the connected
//! frontend and the offline fixture runner.
use crossterm::event::{self, Event};
use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

pub struct Terminal;
impl Terminal {
    pub fn enter() -> Result<Self, String> {
        crossterm::terminal::enable_raw_mode().map_err(|e| e.to_string())?;
        // A later command may fail after entering the alternate screen; own the
        // cleanup before sending any commands to the terminal.
        let guard = Self;
        if let Err(error) = crossterm::execute!(
            io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            event::EnableBracketedPaste,
            event::EnableMouseCapture
        ) {
            return Err(error.to_string());
        }
        Ok(guard)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            io::stdout(),
            event::DisableMouseCapture,
            event::DisableBracketedPaste,
            crossterm::terminal::LeaveAlternateScreen
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
pub struct Reader {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub fn events() -> (Reader, mpsc::Receiver<Result<Event, String>>) {
    let (sender, receiver) = mpsc::channel(64);
    let stop = Arc::new(AtomicBool::new(false));
    let reading = stop.clone();
    let thread = std::thread::spawn(move || {
        while !reading.load(Ordering::Relaxed) {
            let mut pending = match event::poll(Duration::from_millis(50)) {
                Ok(false) => continue,
                Ok(true) => event::read().map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            loop {
                if reading.load(Ordering::Relaxed) {
                    return;
                }
                match sender.try_send(pending) {
                    Ok(()) => break,
                    Err(mpsc::error::TrySendError::Full(event)) => {
                        pending = event;
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                }
            }
        }
    });
    (
        Reader {
            stop,
            thread: Some(thread),
        },
        receiver,
    )
}
/// Shared keyboard path after terminal translation and any modal dialog.
/// The retained selection and semantic panel get first refusal; then the
/// client's editor/picker handles the key.
pub fn route_key(
    screen: &mut crate::Screen,
    retained: &mut crate::retained::Retained,
    view: &misa_proto::Node,
    key: crate::Key,
) -> crate::KeyOut {
    retained
        .selection_key(screen, &key)
        .or_else(|| screen.panel_key(view, &key))
        .unwrap_or_else(|| screen.key(key))
}

pub fn paint(
    writer: &mut impl Write,
    output: &mut crate::output::Output,
    screen: &crate::Screen,
    frame: crate::chrome::Frame,
) -> Result<(), String> {
    output
        .paint(writer, &frame.lines, screen.width as usize, &frame.images)
        .map_err(|e| e.to_string())?;
    write!(
        writer,
        "\x1b[{};{}H",
        frame.cursor_row + 1,
        frame.cursor_column + 1
    )
    .and_then(|_| writer.flush())
    .map_err(|e| e.to_string())
}
