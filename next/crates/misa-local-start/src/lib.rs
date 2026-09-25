//! Opt-in native frontend cold start. Never owns or stops a daemon once spawned.
#![cfg(unix)]

use std::{
    fs::{File, OpenOptions},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);

/// Discover first, serialize cold starts across processes, then wait for a *live*
/// advertisement. Returning never implies ownership of the daemon process.
pub async fn discover_or_start() -> Result<Vec<String>, String> {
    run(&System, WAIT).await
}

trait Operations {
    type Guard;
    type Process;
    async fn discover(&self) -> Result<Vec<String>, String>;
    async fn lock(&self, until: Instant) -> Result<Self::Guard, String>;
    fn launch(&self) -> Result<Self::Process, String>;
    fn exited(&self, child: &mut Self::Process) -> Result<Option<String>, String>;
}

async fn run<O: Operations>(ops: &O, wait: Duration) -> Result<Vec<String>, String> {
    let until = Instant::now() + wait;
    let found = tokio::time::timeout(wait, ops.discover())
        .await
        .map_err(|_| "Timed out discovering local daemons")??;
    if !found.is_empty() {
        return Ok(found);
    }
    let _guard = ops.lock(until).await?;
    let found = tokio::time::timeout(
        until.saturating_duration_since(Instant::now()),
        ops.discover(),
    )
    .await
    .map_err(|_| "Timed out rediscovering local daemons")??;
    if !found.is_empty() {
        return Ok(found);
    }
    if Instant::now() >= until {
        return Err("Timed out waiting for local daemon start lock".into());
    }
    let mut child = ops.launch()?;
    loop {
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Timed out waiting for a local daemon advertisement".into());
        }
        let found = tokio::time::timeout(remaining, ops.discover())
            .await
            .map_err(|_| "Timed out waiting for a local daemon advertisement")??;
        if !found.is_empty() {
            return Ok(found);
        }
        if let Some(status) = ops.exited(&mut child)? {
            return Err(format!("Local daemon exited before advertising: {status}"));
        }
        tokio::time::sleep(POLL.min(until.saturating_duration_since(Instant::now()))).await;
    }
}

/// Keep the child reaped even if it exits after the frontend stops waiting for
/// readiness. Dropping a Child handle alone leaves a zombie while we remain alive.
struct DaemonChild(Option<Child>);
impl Drop for DaemonChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

struct System;
impl Operations for System {
    type Guard = File;
    type Process = DaemonChild;

    async fn discover(&self) -> Result<Vec<String>, String> {
        misa_transport::local::discover().await
    }

    async fn lock(&self, until: Instant) -> Result<File, String> {
        let dir = misa_transport::local::ensure_directory()?;
        tokio::task::spawn_blocking(move || {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(dir.join("start.lock"))
                .map_err(|error| format!("Cannot open local daemon start lock: {error}"))?;
            let meta = file.metadata().map_err(|error| error.to_string())?;
            // A malicious existing lock file must not become an arbitrary write target.
            if !meta.is_file()
                || meta.uid() != unsafe { libc::geteuid() }
                || meta.mode() & 0o777 != 0o600
                || meta.nlink() != 1
            {
                return Err("Local daemon start lock must be a private regular file".into());
            }
            loop {
                // flock is released when File drops, even on error. Do not unlink it:
                // unlinking a lock allows two waiters to lock different inodes.
                if unsafe {
                    libc::flock(
                        std::os::fd::AsRawFd::as_raw_fd(&file),
                        libc::LOCK_EX | libc::LOCK_NB,
                    )
                } == 0
                {
                    return Ok(file);
                }
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                    return Err(format!("Cannot lock local daemon start: {error}"));
                }
                if Instant::now() >= until {
                    return Err("Timed out waiting for local daemon start lock".into());
                }
                std::thread::sleep(POLL.min(until.saturating_duration_since(Instant::now())));
            }
        })
        .await
        .map_err(|error| error.to_string())?
    }

    fn launch(&self) -> Result<DaemonChild, String> {
        let binary = daemon_binary()?;
        let mut command = Command::new(binary);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Do not pass --no-relay: an overridden daemon may not implement it.
        // Detached process lifetime is independent of the invoking frontend.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command
            .spawn()
            .map(|child| DaemonChild(Some(child)))
            .map_err(|error| format!("Cannot start local daemon: {error}"))
    }

    fn exited(&self, child: &mut DaemonChild) -> Result<Option<String>, String> {
        child
            .0
            .as_mut()
            .expect("child remains until dropped")
            .try_wait()
            .map(|status| status.map(|s| s.to_string()))
            .map_err(|error| error.to_string())
    }
}

fn daemon_binary() -> Result<PathBuf, String> {
    if let Some(binary) = std::env::var_os("MISA_DAEMON_BIN") {
        if binary.is_empty() {
            return Err("MISA_DAEMON_BIN is empty".into());
        }
        return Ok(PathBuf::from(binary));
    }
    let sibling = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name("misa-daemon");
    if sibling
        .metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    {
        return Ok(sibling);
    }
    Ok(PathBuf::from("misa-daemon"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct State {
        calls: usize,
        starts: usize,
        exits: bool,
        appear_at: usize,
        delay_first: Duration,
    }
    struct Mock(Arc<Mutex<State>>);
    impl Operations for Mock {
        type Guard = ();
        type Process = ();
        async fn discover(&self) -> Result<Vec<String>, String> {
            let (calls, appear_at, delay) = {
                let mut state = self.0.lock().unwrap();
                state.calls += 1;
                (state.calls, state.appear_at, state.delay_first)
            };
            if calls == 1 {
                tokio::time::sleep(delay).await;
            }
            Ok(if calls >= appear_at {
                vec!["live".into()]
            } else {
                vec![]
            })
        }
        async fn lock(&self, _: Instant) -> Result<(), String> {
            Ok(())
        }
        fn launch(&self) -> Result<(), String> {
            self.0.lock().unwrap().starts += 1;
            Ok(())
        }
        fn exited(&self, _: &mut ()) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().exits.then(|| "exit 1".into()))
        }
    }
    #[tokio::test]
    async fn discovery_skips_launch_even_after_lock() {
        for appear_at in [1, 2] {
            let state = Arc::new(Mutex::new(State {
                appear_at,
                ..Default::default()
            }));
            assert_eq!(run(&Mock(state.clone()), WAIT).await.unwrap(), ["live"]);
            assert_eq!(state.lock().unwrap().starts, 0);
        }
    }
    #[tokio::test]
    async fn waits_for_live_advertisement_or_early_exit() {
        let state = Arc::new(Mutex::new(State {
            appear_at: 4,
            ..Default::default()
        }));
        assert_eq!(run(&Mock(state.clone()), WAIT).await.unwrap(), ["live"]);
        assert_eq!(state.lock().unwrap().starts, 1);
        let state = Arc::new(Mutex::new(State {
            appear_at: usize::MAX,
            exits: true,
            ..Default::default()
        }));
        assert!(
            run(&Mock(state.clone()), WAIT)
                .await
                .unwrap_err()
                .contains("exited")
        );
        assert_eq!(state.lock().unwrap().starts, 1);
    }
    #[tokio::test]
    async fn initial_discovery_is_part_of_the_deadline() {
        let state = Arc::new(Mutex::new(State {
            delay_first: Duration::from_millis(100),
            ..Default::default()
        }));
        assert!(
            run(&Mock(state.clone()), Duration::from_millis(5))
                .await
                .unwrap_err()
                .contains("Timed out discovering")
        );
        assert_eq!(state.lock().unwrap().starts, 0);
    }
    #[tokio::test]
    async fn deadline_is_bounded_and_does_not_stop_child() {
        let state = Arc::new(Mutex::new(State {
            appear_at: usize::MAX,
            ..Default::default()
        }));
        assert!(
            run(&Mock(state.clone()), Duration::from_millis(5))
                .await
                .unwrap_err()
                .contains("Timed out")
        );
        assert_eq!(state.lock().unwrap().starts, 1);
    }
}
