//! Temporary instrumentation for the scroll defect.
//!
//! Every viewport decision, reader writeback and row lookup appends one line to
//! `/tmp/misa-tui-scroll.log`, so a reproduction is a trace instead of a guess.
//! The log is truncated when the process starts and stops accepting lines past
//! [`LIMIT_BYTES`]. Delete this module once the forced movement is explained.

use std::io::Write as _;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
};

const LIMIT_BYTES: u64 = 64 * 1024 * 1024;
const PATH: &str = "/tmp/misa-tui-scroll.log";

static SINK: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
static WRITTEN: AtomicU64 = AtomicU64::new(0);

fn sink() -> Option<&'static Mutex<std::fs::File>> {
    SINK.get_or_init(|| {
        let file = std::fs::File::create(PATH).ok()?;
        Some(Mutex::new(file))
    })
    .as_ref()
}

/// One line per event. Tracing must never decide anything or panic.
pub fn log(event: &str) {
    let Some(file) = sink() else {
        return;
    };
    if WRITTEN.load(Ordering::Relaxed) > LIMIT_BYTES {
        return;
    }
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = file.lock() {
        let line = format!("{ms} {event}\n");
        WRITTEN.fetch_add(line.len() as u64, Ordering::Relaxed);
        let _ = file.write_all(line.as_bytes());
    }
}
