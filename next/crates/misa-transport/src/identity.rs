//! Durable client identity: a pairing belongs to a key, not to a process run.
use std::{io::Write, path::{Path, PathBuf}};
use iroh::SecretKey;

pub fn client_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') { return Err("Invalid client identity name".into()); }
    let root = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or("No state directory for client identity")?;
    Ok(root.join("misa/identities").join(name))
}

pub fn load(path: &Path) -> Result<SecretKey, String> {
    fn read(path: &Path) -> Result<SecretKey, String> {
        let bytes: [u8; 32] = std::fs::read(path).map_err(|e| e.to_string())?.try_into().map_err(|_| "Invalid identity file length")?;
        Ok(SecretKey::from_bytes(&bytes))
    }
    if path.exists() { return read(path); }
    let parent = path.parent().ok_or("Identity path needs a parent")?;
    let mut directory = std::fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; directory.mode(0o700); }
    directory.create(parent).map_err(|e| e.to_string())?;
    let key = SecretKey::generate();
    let temporary = parent.join(format!(".identity-{}", key.public()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(&key.to_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        match std::fs::hard_link(&temporary, path) {
            Ok(()) => Ok(key),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read(path),
            Err(error) => Err(error.to_string()),
        }
    })();
    let _ = std::fs::remove_file(temporary);
    result
}
