//! The tools the shipped kernel can run.
//!
//! A tool *is* a capability, which is why it lives here and not in the session:
//! the session declares a tool to a model (a name, a description, a schema — that
//! is policy) and the kernel is what actually touches a filesystem or starts a
//! process. The old system drew the same line, in Fennel and Zig instead of two
//! Rust modules.
//!
//! # Bounds, and what they are not
//!
//! Every tool is bounded: a read is at most 1 MiB, a write at most 1 MiB, and a
//! command is stopped at its deadline and answered with a bounded tail of the file
//! its output went to. Those are limits on what a *model* can accidentally do, not a
//! sandbox. The previous system said the same thing about its own file and process
//! effects, and it is worth repeating: filesystem and process isolation are
//! properties of the environment that launched the daemon, not of a tool.
//!
//! # A process is not a call with a short leash
//!
//! The one tool whose *timing* is part of its design is the shell. A command that is
//! quiet has not failed, and the previous system's shell — which died when a command
//! stopped printing — is exactly what makes a build or a test run painful to watch.
//! So a command's output goes to a file, the call answers after `wait_ms` whether or
//! not the command is done, a background command is reported when it finishes, and
//! the owner can cancel its processes when it closes. See [`Shell`].
//!
//! # A failure is a result
//!
//! Nothing here returns `Err` for a missing file or a failing command. A tool's
//! problem is a fact the model should see and reason about, which is why the
//! previous system settled on one close-out sentence for every tool and why these
//! return text either way.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use misa_value::Value;
use tokio::sync::mpsc;

use crate::{KernelEvent, Tool};

/// The largest file a tool will read or write.
pub const MAX_BYTES: usize = 1024 * 1024;

/// How long a shell call blocks before it answers "still running".
const DEFAULT_WAIT_MS: i64 = 15_000;
/// The longest a caller may ask one call to block.
const MAX_WAIT_MS: i64 = 120_000;
/// How long a command may run before the kernel stops it. Ten minutes, because a build, a
/// test suite, and a download are all things a minute is not enough for.
const DEFAULT_TIMEOUT_MS: i64 = 600_000;
/// The shortest deadline a caller may ask for: `timeout_ms: 1` should not be a way to kill
/// something by accident.
const MIN_TIMEOUT_MS: i64 = 1_000;
/// The longest, so a forgotten command cannot outlive the session that started it by a day.
const MAX_TIMEOUT_MS: i64 = 3_600_000;
/// How much of a log one answer carries.
const TAIL_BYTES: u64 = 16 * 1024;

/// Where a daemon with no data directory keeps what its shell runs.
///
/// A temporary directory, because a daemon with no data directory is a daemon whose logs are
/// not expected to outlive it — and the logs of a test are worth having while it runs.
pub fn default_shell_dir() -> PathBuf {
    std::env::temp_dir().join(format!("misa-shell-{}", std::process::id()))
}

/// The tools the shipped kernel installs, in the order a model sees them.
///
/// `dir` is where the shell keeps its logs and `events` is where it reports a command that
/// finishes after the call that started it has already answered.
pub fn shipped(dir: &Path, events: &mpsc::UnboundedSender<KernelEvent>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadFile),
        Arc::new(WriteFile),
        Arc::new(ListDirectory),
        Arc::new(Shell::at(dir, events.clone())),
        Arc::new(Echo),
    ]
}

/// Print the arguments back, unchanged.
///
/// The smallest tool there is, and it is kept because it is the one a tool-calling
/// test needs: it has no side effect, so a test can assert on a tool round trip
/// without writing to a filesystem.
pub struct Echo;

#[async_trait]
impl Tool for Echo {
    fn name(&self) -> &str {
        "echo"
    }

    async fn run(&self, args: &Value) -> Result<String, String> {
        Ok(match args {
            Value::Str(text) => text.to_string(),
            other => format!("{other}"),
        })
    }
}

fn arg_text(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn arg_int(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

/// Whether a path an argument named is usable, and why not if it is not.
///
/// An empty path is refused rather than silently meaning the working directory: a
/// model that forgot an argument should be told, not given a directory listing of
/// somewhere it did not ask about.
fn path_of(args: &Value) -> Result<PathBuf, String> {
    let text = arg_text(args, "path");
    if text.trim().is_empty() {
        return Err("no `path` was given".into());
    }
    Ok(PathBuf::from(text))
}

/// Cut text to a bound on a character boundary, saying that it was cut.
fn bounded(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… {} bytes elided", &text[..end], text.len() - end)
}

/// Read a file, optionally a window of its lines.
pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    async fn run(&self, args: &Value) -> Result<String, String> {
        // A missing argument is a result the model should see, which is the same rule
        // every other tool outcome follows.
        let path = match path_of(args) {
            Ok(path) => path,
            Err(reason) => return Ok(reason),
        };
        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            // A binary file is worth saying so about rather than failing on: the
            // model can then decide to use the shell instead.
            Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
                return Ok(format!(
                    "{} is not text; read it another way",
                    path.display()
                ));
            }
            Err(err) => return Ok(format!("could not read {}: {err}", path.display())),
        };
        let first = arg_int(args, "start_line").unwrap_or(1).max(1) as usize;
        let window = arg_int(args, "max_lines").map(|count| count.clamp(1, 20_000) as usize);
        let lines: Vec<&str> = text.lines().collect();
        let start = (first - 1).min(lines.len());
        let end = window.map_or(lines.len(), |count| (start + count).min(lines.len()));
        let mut out = String::new();
        for (offset, line) in lines[start..end].iter().enumerate() {
            // A line number the model can quote back, which is what makes an
            // anchored edit possible.
            out.push_str(&format!("{:>5}  {line}\n", start + offset + 1));
        }
        if end < lines.len() {
            out.push_str(&format!("… {} more lines\n", lines.len() - end));
        }
        Ok(bounded(&out, MAX_BYTES))
    }
}

/// Write a file, creating its directory if it does not exist.
pub struct WriteFile;

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }

    async fn run(&self, args: &Value) -> Result<String, String> {
        // A missing argument is a result the model should see, which is the same rule
        // every other tool outcome follows.
        let path = match path_of(args) {
            Ok(path) => path,
            Err(reason) => return Ok(reason),
        };
        let content = arg_text(args, "content");
        if content.len() > MAX_BYTES {
            return Ok(format!(
                "refusing to write {} bytes; the bound is {MAX_BYTES}",
                content.len()
            ));
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && let Err(err) = tokio::fs::create_dir_all(parent).await
        {
            return Ok(format!("could not create {}: {err}", parent.display()));
        }
        match tokio::fs::write(&path, content.as_bytes()).await {
            Ok(()) => Ok(format!(
                "wrote {} bytes to {}",
                content.len(),
                path.display()
            )),
            Err(err) => Ok(format!("could not write {}: {err}", path.display())),
        }
    }
}

/// List a directory.
pub struct ListDirectory;

#[async_trait]
impl Tool for ListDirectory {
    fn name(&self) -> &str {
        "list_directory"
    }

    async fn run(&self, args: &Value) -> Result<String, String> {
        // A missing argument is a result the model should see, which is the same rule
        // every other tool outcome follows.
        let path = match path_of(args) {
            Ok(path) => path,
            Err(reason) => return Ok(reason),
        };
        let mut entries = match tokio::fs::read_dir(&path).await {
            Ok(entries) => entries,
            Err(err) => return Ok(format!("could not list {}: {err}", path.display())),
        };
        // Sorted, because a directory listing that changes order between two calls
        // makes a model's diff of its own output unreadable.
        let mut names: Vec<String> = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = match entry.file_type().await {
                Ok(kind) if kind.is_dir() => "/",
                Ok(kind) if kind.is_symlink() => "@",
                _ => "",
            };
            names.push(format!("{name}{kind}"));
        }
        names.sort();
        Ok(bounded(&names.join("\n"), MAX_BYTES))
    }
}

/// Run a shell command, with everything it prints going into a file.
///
/// # Why it is not "capture stdout and wait"
///
/// A command that is quiet for a minute has not failed, and a tool that gives up on it is a
/// tool nobody can build anything with: the previous system's shell died whenever a command
/// stopped printing, which is what makes a build, a test run, or a download painful to watch.
/// So a command here is never a short-leashed blocking call:
///
/// - its output goes to a file from the first byte, so nothing is lost while it runs;
/// - the call blocks for `wait_ms` and then *answers anyway*, leaving the command running;
/// - `background: true` says so up front, and the answer comes back immediately;
/// - the process is killed at `timeout_ms` or when its owning session closes;
/// - when a command is left running the answer carries its pid and the file to tail, and the
///   session is told when it finishes, so a turn that ended long ago still learns the outcome.
///
/// The point of the file is that the answer and the process are decoupled. The model is told
/// where to look instead of being handed a snapshot it cannot refresh, and a command that
/// takes ten minutes costs one tool call rather than ten minutes of blocked turn.
pub struct Shell {
    /// Where the logs are written.
    dir: PathBuf,
    /// Where a command that outlives its call is reported.
    events: mpsc::UnboundedSender<KernelEvent>,
    /// Names the next run, so two calls never share a log file.
    seq: AtomicU64,
}

fn kill_group(pid: u32) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid) {
        if pid > 0 {
            // The child created its own process group and has not yet been reaped.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

impl Shell {
    /// A shell whose logs live in `dir` and whose finished commands are reported to `events`.
    pub fn at(dir: impl Into<PathBuf>, events: mpsc::UnboundedSender<KernelEvent>) -> Shell {
        Shell {
            dir: dir.into(),
            events,
            seq: AtomicU64::new(0),
        }
    }

    /// A name for one run's log: numbered, and readable enough to find in a directory.
    fn next_name(&self, command: &str) -> String {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        format!("p{seq}-{}", slug(command))
    }
}

#[async_trait]
impl Tool for Shell {
    fn name(&self) -> &str {
        "shell"
    }

    async fn run(&self, args: &Value) -> Result<String, String> {
        self.run_in(
            args,
            crate::ToolContext {
                reports: self.events.clone(),
                cancelled: None,
            },
        )
        .await
    }

    async fn run_in(&self, args: &Value, context: crate::ToolContext) -> Result<String, String> {
        if context
            .cancelled
            .as_ref()
            .is_some_and(|cancelled| *cancelled.borrow())
        {
            return Err("owning session closed".into());
        }
        let command = arg_text(args, "command");
        if command.trim().is_empty() {
            return Ok("no `command` was given".into());
        }
        let background = args
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let wait = Duration::from_millis(
            arg_int(args, "wait_ms")
                .unwrap_or(DEFAULT_WAIT_MS)
                .clamp(0, MAX_WAIT_MS) as u64,
        );
        let deadline = Duration::from_millis(
            arg_int(args, "timeout_ms")
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS) as u64,
        );

        let log = self.dir.join(format!("{}.log", self.next_name(&command)));
        if let Err(err) = tokio::fs::create_dir_all(&self.dir).await {
            return Ok(format!("could not make a place for the log: {err}"));
        }
        // Appended by everyone who writes here: the command, its children, and the closing line
        // the watcher adds when it is over.
        let file = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
        {
            Ok(file) => file,
            Err(err) => return Ok(format!("could not open {}: {err}", log.display())),
        };
        let stderr = match file.try_clone() {
            Ok(stderr) => stderr,
            Err(err) => return Ok(format!("could not redirect the output: {err}")),
        };
        // `sh -lc` explicitly, never a string handed to a shell by a library that meant to exec
        // a program. The previous system put the same translation in its tool and the same note
        // beside it: isolation belongs to the launcher, not the tool.
        let mut process = tokio::process::Command::new("sh");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            process.as_std_mut().process_group(0);
        }
        let spawned = process
            .arg("-lc")
            .arg(&command)
            .stdout(std::process::Stdio::from(file))
            .stderr(std::process::Stdio::from(stderr))
            .stdin(std::process::Stdio::null())
            // Not killed when this call returns, because that is the whole point: the call
            // answers, the command keeps going.
            .kill_on_drop(false)
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(err) => return Ok(format!("could not run the command: {err}")),
        };
        let pid = child.id().unwrap_or_default();
        let started = std::time::Instant::now();

        // One of the two paths says a command finished, never both: an answer the model already
        // has is not worth a second, later notice. The call path claims the finish by sending
        // on this channel; if it never does — backgrounded, or answered while still running —
        // the watcher reports it instead.
        let (claimed_tx, claimed_rx) = tokio::sync::oneshot::channel::<()>();
        let (status_tx, status_rx) = tokio::sync::oneshot::channel::<Outcome>();
        let events = context.reports;
        let mut cancelled = context.cancelled;
        let report = log.clone();
        let named = command.clone();
        tokio::spawn(async move {
            let (waited, killed) = tokio::select! {
                status = child.wait() => (status, false),
                _ = async { match cancelled.as_mut() {Some(cancelled)=>{let _=cancelled.wait_for(|value|*value).await;},None=>std::future::pending::<()>().await} } => {
                    kill_group(pid);
                    let _ = child.start_kill();
                    (child.wait().await, true)
                },
                _ = tokio::time::sleep(deadline) => {
                    kill_group(pid);
                    let _ = child.start_kill();
                    (child.wait().await, true)
                }
            };
            let outcome = Outcome {
                exit: waited.ok().and_then(|status| status.code()),
                killed,
                seconds: started.elapsed().as_secs_f64(),
            };
            close_the_log(&report, &outcome);
            let _ = status_tx.send(outcome);
            if claimed_rx.await.is_err() {
                let _ = events.send(KernelEvent::ProcessFinished {
                    pid,
                    command: named,
                    exit: outcome.exit,
                    killed: outcome.killed,
                    seconds: outcome.seconds,
                    log: report.to_string_lossy().to_string(),
                });
            }
        });

        // Every answer reads the log *after* the moment it is about: a command that finished in
        // three milliseconds has its output by the time its status arrives, and reading earlier
        // would answer with the emptiness of a file nobody had written to yet.
        if background {
            let (tail, bytes) = tail_of(&log).await;
            let mut out = running(
                &format!("started in the background, pid {pid}"),
                &log,
                bytes,
                &tail,
            );
            out.push_str("the session will report when it finishes.\n");
            return Ok(out);
        }
        match tokio::time::timeout(wait, status_rx).await {
            Ok(Ok(outcome)) => {
                // Reported here, so the watcher stays quiet.
                let _ = claimed_tx.send(());
                let (tail, bytes) = tail_of(&log).await;
                Ok(finished(&log, &outcome, bytes, &tail))
            }
            Ok(Err(_)) => {
                Ok("the command could not be waited for, so its status is unknown".into())
            }
            Err(_) => {
                let (tail, bytes) = tail_of(&log).await;
                let mut out = running(
                    &format!(
                        "still running, pid {pid}, after {}",
                        seconds_text(wait.as_secs_f64())
                    ),
                    &log,
                    bytes,
                    &tail,
                );
                out.push_str(&format!(
                    "raise `wait_ms` to block longer, or read the log when you want to know more; \
                     it is stopped at {} unless the process ends first.\n",
                    seconds_text(deadline.as_secs_f64())
                ));
                Ok(out)
            }
        }
    }
}

/// What became of a command, as the watcher saw it.
#[derive(Clone, Copy, Debug)]
struct Outcome {
    exit: Option<i32>,
    /// Whether this kernel stopped it at its deadline rather than the command ending.
    killed: bool,
    seconds: f64,
}

/// One line saying how it ended, appended to the log so `tail -f` shows the end of a command
/// and not only its output.
fn close_the_log(log: &Path, outcome: &Outcome) {
    use std::io::Write as _;
    let line = format!("\n--- misa: {} ---\n", ending(outcome));
    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(log) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// How a command ended, in words.
fn ending(outcome: &Outcome) -> String {
    match (outcome.killed, outcome.exit) {
        (true, _) => format!(
            "stopped at its deadline after {}",
            seconds_text(outcome.seconds)
        ),
        (false, Some(code)) => format!("exit {code} after {}", seconds_text(outcome.seconds)),
        (false, None) => format!("ended by a signal after {}", seconds_text(outcome.seconds)),
    }
}

/// A finished command's answer: its status, where the whole output is, and the end of it.
fn finished(log: &Path, outcome: &Outcome, bytes: u64, tail: &str) -> String {
    let mut out = format!(
        "{}\nlog: {} ({})\n",
        ending(outcome),
        log.display(),
        bytes_text(bytes)
    );
    out.push_str(&tail_section(tail, bytes));
    out
}

/// A command that is still going: its pid, where the output is, and what there is so far.
fn running(status: &str, log: &Path, bytes: u64, tail: &str) -> String {
    let mut out = format!("{status}\nlog: {} ({})\n", log.display(), bytes_text(bytes));
    out.push_str(&tail_section(tail, bytes));
    out
}

/// The output an answer carries, and whether it is the whole of it.
///
/// The end rather than the beginning, because the end is where a failure says what went wrong —
/// and the file is named either way, so a model that wants the first line can read it.
fn tail_section(tail: &str, bytes: u64) -> String {
    if tail.trim().is_empty() {
        return "(the command has printed nothing yet)\n".to_string();
    }
    let mut out = if bytes > TAIL_BYTES {
        format!(
            "--- last {} of {} ---\n",
            bytes_text(TAIL_BYTES),
            bytes_text(bytes)
        )
    } else {
        "--- output ---\n".to_string()
    };
    out.push_str(tail);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// The end of a log, and how large the whole of it is.
async fn tail_of(log: &Path) -> (String, u64) {
    let path = log.to_path_buf();
    tokio::task::spawn_blocking(move || read_tail(&path))
        .await
        .unwrap_or_else(|_| (String::new(), 0))
}

/// The last [`TAIL_BYTES`] of a file, without reading the whole of it.
fn read_tail(path: &Path) -> (String, u64) {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return (String::new(), 0);
    };
    let bytes = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let start = bytes.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return (String::new(), bytes);
    }
    let mut buffer = Vec::new();
    if file.read_to_end(&mut buffer).is_err() {
        return (String::new(), bytes);
    }
    let text = String::from_utf8_lossy(&buffer).to_string();
    // A tail that begins in the middle of a line is not a line; `tail` throws it away and so
    // does this, so the first thing in the answer is always something that was written whole.
    let text = if start == 0 {
        text
    } else {
        text.split_once('\n')
            .map(|(_, rest)| rest.to_string())
            .unwrap_or(text)
    };
    (text, bytes)
}

/// A byte count, as a person would say it.
fn bytes_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// A duration, as a person would say it.
fn seconds_text(seconds: f64) -> String {
    if seconds < 1.0 {
        format!("{}ms", (seconds * 1000.0).round() as i64)
    } else if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        format!("{}m{:.0}s", (seconds / 60.0).floor() as i64, seconds % 60.0)
    }
}

/// The first few words of a command, as something a file name can carry.
///
/// Only so a log file can be found by eye in a directory of them; the number in front is what
/// makes it unique.
fn slug(command: &str) -> String {
    let mut out = String::new();
    for word in command.split_whitespace().take(4) {
        let clean: String = word
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '.')
            .take(12)
            .collect();
        if clean.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&clean);
    }
    if out.is_empty() {
        return "command".to_string();
    }
    out.chars().take(48).collect()
}

/// Whether a path names something that exists, for a test or a diagnostic.
pub async fn exists(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(std::path::PathBuf);

    impl Temp {
        async fn new(name: &str) -> Temp {
            let path =
                std::env::temp_dir().join(format!("misa-tools-{name}-{}", std::process::id()));
            let _ = tokio::fs::remove_dir_all(&path).await;
            tokio::fs::create_dir_all(&path)
                .await
                .expect("a temp directory");
            Temp(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let path = self.0.clone();
            tokio::spawn(async move {
                let _ = tokio::fs::remove_dir_all(path).await;
            });
        }
    }

    #[tokio::test]
    async fn a_write_then_a_read_returns_numbered_lines() {
        let temp = Temp::new("roundtrip").await;
        let file = temp.0.join("nested/notes.txt");
        let written = WriteFile
            .run(&Value::map([
                ("path", Value::str(file.to_string_lossy())),
                ("content", Value::str("first\nsecond\nthird\n")),
            ]))
            .await
            .expect("a result");
        assert!(written.contains("wrote 19 bytes"), "{written}");

        let read = ReadFile
            .run(&Value::map([("path", Value::str(file.to_string_lossy()))]))
            .await
            .expect("a result");
        assert!(read.contains("    1  first"), "{read}");
        assert!(read.contains("    3  third"), "{read}");
    }

    #[tokio::test]
    async fn a_read_window_says_what_it_left_out() {
        let temp = Temp::new("window").await;
        let file = temp.0.join("many.txt");
        let content: String = (1..=20).map(|index| format!("line {index}\n")).collect();
        WriteFile
            .run(&Value::map([
                ("path", Value::str(file.to_string_lossy())),
                ("content", Value::str(&content)),
            ]))
            .await
            .unwrap();
        let read = ReadFile
            .run(&Value::map([
                ("path", Value::str(file.to_string_lossy())),
                ("start_line", Value::Int(5)),
                ("max_lines", Value::Int(2)),
            ]))
            .await
            .unwrap();
        assert!(read.contains("    5  line 5"), "{read}");
        assert!(read.contains("    6  line 6"), "{read}");
        assert!(!read.contains("line 7"), "{read}");
        assert!(read.contains("more lines"), "{read}");
    }

    #[tokio::test]
    async fn a_missing_file_is_a_result_and_not_a_failure() {
        let temp = Temp::new("missing").await;
        let read = ReadFile
            .run(&Value::map([(
                "path",
                Value::str(temp.0.join("nope").to_string_lossy()),
            )]))
            .await
            .expect("a result, not an error");
        assert!(read.contains("could not read"), "{read}");
    }

    #[tokio::test]
    async fn a_path_that_was_not_given_is_refused_rather_than_guessed() {
        let read = ReadFile.run(&Value::Null).await.expect("a result");
        assert!(read.contains("no `path`"), "{read}");
    }

    #[tokio::test]
    async fn a_directory_listing_is_sorted_and_marks_directories() {
        let temp = Temp::new("listing").await;
        tokio::fs::create_dir(temp.0.join("beta")).await.unwrap();
        tokio::fs::write(temp.0.join("alpha.txt"), b"x")
            .await
            .unwrap();
        let listing = ListDirectory
            .run(&Value::map([(
                "path",
                Value::str(temp.0.to_string_lossy()),
            )]))
            .await
            .unwrap();
        let lines: Vec<&str> = listing.lines().collect();
        assert_eq!(lines, vec!["alpha.txt", "beta/"]);
    }

    /// A shell whose logs go somewhere fresh, and the far end of what it reports.
    ///
    /// The receiver is returned because a command that outlives its call is *only* observable
    /// through it: the answer says "still running", and the report says how it ended.
    fn shell_named(name: &str) -> (Shell, mpsc::UnboundedReceiver<KernelEvent>) {
        let (events, reports) = mpsc::unbounded_channel();
        let dir =
            std::env::temp_dir().join(format!("misa-tools-shell-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Shell::at(dir, events), reports)
    }

    /// Wait for one report, which is the only way a background command has of saying anything.
    async fn report(reports: &mut mpsc::UnboundedReceiver<KernelEvent>) -> KernelEvent {
        tokio::time::timeout(Duration::from_secs(10), reports.recv())
            .await
            .expect("a report")
            .expect("the channel is open")
    }

    /// The log a shell answer named, so a test can read what the command actually printed.
    fn log_of(answer: &str) -> PathBuf {
        let line = answer
            .lines()
            .find(|line| line.starts_with("log: "))
            .expect("a log line");
        PathBuf::from(
            line.trim_start_matches("log: ")
                .split(" (")
                .next()
                .expect("a path"),
        )
    }

    #[tokio::test]
    async fn a_shell_command_reports_its_status_its_output_and_where_the_output_is() {
        let (shell, _reports) = shell_named("finished");
        let result = shell
            .run(&Value::map([(
                "command",
                Value::str("echo hello; echo oops >&2"),
            )]))
            .await
            .expect("a result");
        assert!(result.contains("exit 0"), "{result}");
        assert!(result.contains("hello"), "{result}");
        assert!(result.contains("oops"), "{result}");
        // Both streams land in one file, in the order they were written, because that is what a
        // person watching a command sees and what a model reading it back needs.
        let log = std::fs::read_to_string(log_of(&result)).expect("the log");
        assert!(log.contains("hello"), "{log}");
        assert!(log.contains("oops"), "{log}");
        // And the file says how it ended, so `tail -f` shows the end rather than only the output.
        assert!(log.contains("--- misa: exit 0 after"), "{log}");
    }

    #[tokio::test]
    async fn a_command_that_is_still_running_answers_with_its_pid_and_its_log_and_keeps_running() {
        let (shell, mut reports) = shell_named("running");
        let result = shell
            .run(&Value::map([
                (
                    "command",
                    Value::str("echo starting; sleep 1; echo finished"),
                ),
                ("wait_ms", Value::Int(200)),
                ("timeout_ms", Value::Int(5_000)),
            ]))
            .await
            .expect("a result");
        assert!(result.contains("still running"), "{result}");
        assert!(result.contains("pid "), "{result}");
        let log = log_of(&result);
        // The answer carries what there is so far, not nothing: a command that is quiet is not
        // a command that has said nothing.
        assert!(result.contains("starting"), "{result}");

        // It was not killed when the call gave up waiting, which is the whole point.
        let report = report(&mut reports).await;
        match report {
            KernelEvent::ProcessFinished {
                exit,
                killed,
                log: reported,
                command,
                ..
            } => {
                assert_eq!(exit, Some(0));
                assert!(
                    !killed,
                    "a command that finished on its own was reported as killed"
                );
                assert_eq!(command, "echo starting; sleep 1; echo finished");
                assert_eq!(PathBuf::from(reported), log);
            }
            other => panic!("a background command reported `{}`", other.kind()),
        }
        assert!(
            std::fs::read_to_string(&log).unwrap().contains("finished"),
            "the log lost the end"
        );
    }

    #[tokio::test]
    async fn a_background_command_answers_at_once_and_is_reported_when_it_ends() {
        let (shell, mut reports) = shell_named("background");
        let started = std::time::Instant::now();
        let result = shell
            .run(&Value::map([
                ("command", Value::str("echo later; sleep 1; echo done")),
                ("background", Value::Bool(true)),
            ]))
            .await
            .expect("a result");
        assert!(result.contains("started in the background"), "{result}");
        assert!(
            started.elapsed() < Duration::from_millis(900),
            "a background call blocked"
        );
        assert!(result.contains("pid "), "{result}");

        match report(&mut reports).await {
            KernelEvent::ProcessFinished {
                exit,
                killed,
                seconds,
                log,
                ..
            } => {
                assert_eq!(exit, Some(0));
                assert!(!killed);
                assert!(seconds > 0.5, "the report says it took {seconds}s");
                let log = std::fs::read_to_string(log).expect("the log");
                assert!(log.contains("done"), "{log}");
            }
            other => panic!("a background command reported `{}`", other.kind()),
        }
    }

    #[tokio::test]
    async fn a_command_is_stopped_only_at_its_deadline_and_the_log_says_so() {
        let (shell, mut reports) = shell_named("deadline");
        let result = shell
            .run(&Value::map([
                ("command", Value::str("echo working; sleep 30")),
                ("wait_ms", Value::Int(100)),
                ("timeout_ms", Value::Int(1_000)),
            ]))
            .await
            .expect("a result");
        assert!(result.contains("still running"), "{result}");
        let log = log_of(&result);
        match report(&mut reports).await {
            KernelEvent::ProcessFinished {
                killed,
                exit,
                log: reported,
                ..
            } => {
                assert!(
                    killed,
                    "a command past its deadline was not reported as stopped"
                );
                // A signal ends it, and `sh` reports the status of what it waited for; what
                // matters is that the answer says it was the kernel that stopped it.
                let _ = exit;
                assert_eq!(PathBuf::from(reported), log);
            }
            other => panic!("a stopped command reported `{}`", other.kind()),
        }
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("working"), "{text}");
        assert!(text.contains("--- misa: stopped at its deadline"), "{text}");
    }

    #[tokio::test]
    async fn a_lot_of_output_is_answered_with_its_end_and_kept_whole_in_the_file() {
        let (shell, _reports) = shell_named("volume");
        let result = shell
            .run(&Value::map([("command", Value::str("seq 1 40000"))]))
            .await
            .expect("a result");
        assert!(
            result.contains("--- last "),
            "a large answer was not bounded:\n{result}"
        );
        // The end is what an answer carries, because the end is where a failure says what went
        // wrong — and the file is named, so nothing is lost.
        assert!(result.contains("40000"), "{result}");
        assert!(
            !result.contains("\n1\n"),
            "the answer carried the beginning it did not need"
        );
        let log = log_of(&result);
        let bytes = std::fs::metadata(&log).expect("the log").len();
        assert!(bytes > 200_000, "the log is only {bytes} bytes");
    }

    #[tokio::test]
    async fn every_run_gets_its_own_log() {
        let (shell, _reports) = shell_named("names");
        let first = shell
            .run(&Value::map([("command", Value::str("echo one"))]))
            .await
            .expect("a result");
        let second = shell
            .run(&Value::map([("command", Value::str("echo two"))]))
            .await
            .expect("a result");
        assert_ne!(log_of(&first), log_of(&second), "two runs shared a log");
        assert!(
            log_of(&first).to_string_lossy().contains("p1-echo-one"),
            "{first}"
        );
        assert!(
            log_of(&second).to_string_lossy().contains("p2-echo-two"),
            "{second}"
        );
    }

    #[tokio::test]
    async fn a_failing_command_is_a_result_with_its_status() {
        let (shell, _reports) = shell_named("failing");
        let result = shell
            .run(&Value::map([("command", Value::str("exit 3"))]))
            .await
            .unwrap();
        assert!(result.contains("exit 3"), "{result}");
    }

    #[tokio::test]
    async fn a_write_beyond_the_bound_is_refused_before_it_touches_the_disk() {
        let temp = Temp::new("toobig").await;
        let file = temp.0.join("big");
        let content = "x".repeat(MAX_BYTES + 1);
        let result = WriteFile
            .run(&Value::map([
                ("path", Value::str(file.to_string_lossy())),
                ("content", Value::str(&content)),
            ]))
            .await
            .unwrap();
        assert!(result.contains("refusing to write"), "{result}");
        assert!(
            !exists(&file).await,
            "a refused write created the file anyway"
        );
    }

    #[test]
    fn every_shipped_tool_has_a_name_a_model_could_be_told() {
        let (events, _reports) = mpsc::unbounded_channel();
        for tool in shipped(&default_shell_dir(), &events) {
            assert!(!tool.name().is_empty());
            assert!(
                tool.name()
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch == '_'),
                "`{}` is not a name a tool schema would use",
                tool.name()
            );
        }
    }
}
