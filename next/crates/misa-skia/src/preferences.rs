//! Platform path and blocking-I/O boundary for shared presentation preferences.
use misa_client::{
    composition::{Choice, Preferences},
    preference_store::Store,
};
fn path(daemon: &str) -> Result<std::path::PathBuf, String> {
    if daemon.is_empty()
        || daemon.len() > 128
        || !daemon.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Invalid daemon identity for presentation preferences".into());
    }
    Ok(misa_transport::identity::client_path("misa-skia")?
        .with_file_name(format!("misa-skia-presentations-{daemon}.json")))
}
pub async fn load(daemon: String) -> Result<Preferences, String> {
    tokio::task::spawn_blocking(move || Store::new(path(&daemon)?).load())
        .await
        .map_err(|error| error.to_string())?
}
pub async fn save(daemon: String, id: String, choice: Choice) -> Result<(), String> {
    tokio::task::spawn_blocking(move || Store::new(path(&daemon)?).update(id, choice).map(|_| ()))
        .await
        .map_err(|error| error.to_string())?
}

fn appearance_path() -> Result<std::path::PathBuf, String> {
    Ok(misa_transport::identity::client_path("misa-skia")?.with_file_name("misa-skia-appearance"))
}
pub fn appearance() -> crate::appearance::Choice {
    appearance_path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|value| crate::appearance::Choice::parse(value.trim()))
        .unwrap_or_default()
}
/// One local writer preserves click order without doing filesystem work on the window loop.
pub fn appearance_writer(
    proxy: winit::event_loop::EventLoopProxy<crate::connection::Update>,
) -> std::sync::mpsc::SyncSender<crate::appearance::Choice> {
    let (sender, receiver) = std::sync::mpsc::sync_channel::<crate::appearance::Choice>(8);
    std::thread::spawn(move || {
        for choice in receiver {
            if let Err(error) = write_appearance(choice) {
                let _ = proxy.send_event(crate::connection::Update::Notice(format!(
                    "Could not save appearance: {error}"
                )));
            }
        }
    });
    sender
}
fn write_appearance(choice: crate::appearance::Choice) -> Result<(), String> {
    use std::io::Write;
    let path = appearance_path()?;
    let parent = path.parent().ok_or("Appearance path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(choice.name().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        std::fs::File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result.map_err(|error: std::io::Error| error.to_string())
}
