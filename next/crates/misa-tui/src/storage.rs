//! The terminal's filesystem adapter for the kit's presentation memory.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use misa_client::prefs::Storage;

pub struct File {
    path: PathBuf,
}
impl File {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> PathBuf {
        if let Ok(explicit) = std::env::var("MISA_PREFS")
            && !explicit.is_empty()
        {
            return PathBuf::from(explicit);
        }
        let state = std::env::var("XDG_STATE_HOME")
            .ok()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        state.join("misa/client.json")
    }
}

impl Storage for File {
    fn read(&self) -> Result<Option<String>, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("could not read {}: {error}", self.path.display())),
        }
    }

    fn write(&self, text: &str) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not make {}: {error}", parent.display()))?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temporary = self.path.with_extension(format!(
            "writing-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
        let result = file
            .write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .and_then(|_| std::fs::rename(&temporary, &self.path));
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(|error| format!("could not write {}: {error}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_client::Prefs;

    #[test]
    fn a_terminal_saves_private_complete_documents_and_reports_write_failures() {
        let directory = std::env::temp_dir().join(format!("misa-storage-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("client.json");
        let storage = File::at(path.clone());
        let prefs = Prefs {
            draft: "unsent draft".into(),
            ..Prefs::default()
        };
        prefs.save(&storage).unwrap();
        assert_eq!(Prefs::load(&storage), prefs);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(prefs.save(&File::at(directory.clone())).is_err());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
