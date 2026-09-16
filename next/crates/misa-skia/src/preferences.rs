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
