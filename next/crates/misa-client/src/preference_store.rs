//! Process-shared local presentation preferences. Writers merge one choice under
//! a stable lock file; atomic replacement keeps readers on complete documents.
use crate::composition::{Choice, Preferences};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
const LIMIT: usize = 1024 * 1024;
pub struct Store {
    path: PathBuf,
}
impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn load(&self) -> Result<Preferences, String> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Preferences::default());
            }
            Err(error) => return Err(error.to_string()),
        };
        let mut bytes = vec![];
        file.take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Presentation preferences exceed the storage limit".into());
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid presentation preferences: {error}"))
    }
    pub fn update(&self, id: String, choice: Choice) -> Result<Preferences, String> {
        if id.is_empty() || id.len() > 4096 {
            return Err("Invalid presentation preference identity".into());
        }
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        // The lock inode is never renamed or removed, including after failed writes.
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock = private_options()
            .create(true)
            .truncate(false)
            .open(PathBuf::from(lock_path))
            .map_err(|error| error.to_string())?;
        lock.lock().map_err(|error| error.to_string())?;
        let mut preferences = self.load()?;
        preferences.0.insert(id, choice);
        let bytes = serde_json::to_vec(&preferences).map_err(|error| error.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Presentation preferences exceed the storage limit".into());
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temporary = self.path.with_extension(format!(
            "writing-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = private_options().create_new(true).open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &self.path)?;
            #[cfg(unix)]
            File::open(parent)?.sync_all()?;
            Ok::<_, std::io::Error>(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(|error| error.to_string())?;
        Ok(preferences)
    }
}
fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_writer() {
        let Ok(path) = std::env::var("MISA_TEST_PRESENTATIONS") else {
            return;
        };
        let id = std::env::var("MISA_TEST_CHOICE").unwrap();
        Store::new(path.into()).update(id, Choice::Hidden).unwrap();
    }
    #[test]
    fn processes_merge_disjoint_updates_and_corruption_never_discards_choices() {
        let directory =
            std::env::temp_dir().join(format!("misa-preferences-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        let _ = std::fs::remove_file(&path);
        let children = (0..8)
            .map(|n| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "preference_store::tests::child_writer"])
                    .env("MISA_TEST_PRESENTATIONS", &path)
                    .env("MISA_TEST_CHOICE", format!("presentation-{n}"))
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for mut child in children {
            assert!(child.wait().unwrap().success());
        }
        let store = Store::new(path.clone());
        assert_eq!(store.load().unwrap().0.len(), 8);
        std::fs::write(&path, "broken").unwrap();
        assert!(store.update("another".into(), Choice::Auto).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
