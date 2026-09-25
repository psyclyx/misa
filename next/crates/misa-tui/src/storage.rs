//! The connected terminal's filesystem adapter for presentation memory.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use misa_tui_app::{
    PreferencePersistence,
    prefs::{Prefs, Storage},
};

pub struct File {
    path: PathBuf,
}
impl File {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Merge only this surface's changed settings under a stable cross-process lock.
    pub fn update(&self, before: &Prefs, after: &Prefs) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(self.path.with_extension("lock"))
            .map_err(|error| error.to_string())?;
        lock.lock().map_err(|error| error.to_string())?;
        let mut current: serde_json::Value = match self.read()? {
            Some(text) => serde_json::from_str(&text)
                .map_err(|error| format!("Cannot merge invalid preferences: {error}"))?,
            None => serde_json::to_value(Prefs::default()).unwrap(),
        };
        let current = current
            .as_object_mut()
            .ok_or("Preferences must be an object")?;
        let before = serde_json::to_value(before).map_err(|error| error.to_string())?;
        let after = serde_json::to_value(after).map_err(|error| error.to_string())?;
        for (key, value) in after.as_object().unwrap() {
            if before.get(key) == Some(value) {
                continue;
            }
            if matches!(key.as_str(), "drafts" | "frecency") {
                let object = current
                    .entry(key.clone())
                    .or_insert_with(|| serde_json::json!({}))
                    .as_object_mut()
                    .ok_or("Preference map is invalid")?;
                for (id, value) in value.as_object().ok_or("Preference map is invalid")? {
                    if before.get(key).and_then(|value| value.get(id)) != Some(value) {
                        object.insert(id.clone(), value.clone());
                    }
                }
            } else {
                current.insert(key.clone(), value.clone());
            }
        }
        self.write(&serde_json::to_string_pretty(current).map_err(|error| error.to_string())?)
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

impl PreferencePersistence for File {
    fn update(&self, base: &Prefs, next: &Prefs) -> Result<(), String> {
        File::update(self, base, next)
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

    #[test]
    fn independent_writers_merge_drafts_from_stale_snapshots() {
        let directory = std::env::temp_dir().join(format!(
            "misa-draft-writers-{}-{}",
            std::process::id(),
            crate::test_unique_id()
        ));
        let path = directory.join("client.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads = (0..8)
            .map(|index| {
                let (path, barrier) = (path.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let before = Prefs::default();
                    let mut after = before.clone();
                    after
                        .drafts
                        .insert(format!("peer{index}:scope:epoch"), format!("draft{index}"));
                    barrier.wait();
                    File::at(path).update(&before, &after).unwrap();
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        let prefs = Prefs::load(&File::at(path));
        assert_eq!(prefs.drafts.len(), 8);
        for index in 0..8 {
            assert_eq!(
                prefs.drafts[&format!("peer{index}:scope:epoch")],
                format!("draft{index}")
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn connected_screens_merge_stale_drafts_and_reopen_the_same_path() {
        let directory = std::env::temp_dir().join(format!(
            "misa-screen-writers-{}-{}",
            std::process::id(),
            crate::test_unique_id()
        ));
        let path = directory.join("client.json");
        let mut first = crate::ConnectedScreen::remembering(Prefs::default(), path.clone());
        let mut second = crate::ConnectedScreen::remembering(Prefs::default(), path.clone());
        first.ui.enter_draft_scope("first:session:epoch".into());
        first.ui.composer.set_text("first draft");
        first.ui.save();
        second.ui.enter_draft_scope("second:session:epoch".into());
        second.ui.composer.set_text("second draft");
        second.ui.save();
        assert!(first.ui.notice.is_none());
        assert!(second.ui.notice.is_none());
        let stored = Prefs::load(&File::at(path.clone()));
        assert_eq!(stored.drafts["first:session:epoch"], "first draft");
        assert_eq!(stored.drafts["second:session:epoch"], "second draft");
        let mut reopened = crate::ConnectedScreen::remembering(stored, path);
        reopened.ui.enter_draft_scope("first:session:epoch".into());
        assert_eq!(reopened.ui.composer.text(), "first draft");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn screen_reports_failed_saves_and_retries_from_its_original_snapshot() {
        let directory = std::env::temp_dir().join(format!(
            "misa-screen-failure-{}-{}",
            std::process::id(),
            crate::test_unique_id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("client.json");
        std::fs::create_dir(&path).unwrap(); // A directory cannot be atomically replaced by a file.
        let mut screen = crate::ConnectedScreen::remembering(Prefs::default(), path.clone());
        screen.ui.composer.set_text("not yet saved");
        screen.ui.save();
        assert!(
            screen
                .ui
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("client.json"))
        );
        std::fs::remove_dir(&path).unwrap();
        screen.ui.notice = None;
        screen.ui.save();
        assert!(screen.ui.notice.is_none());
        assert_eq!(Prefs::load(&File::at(path)).draft, "not yet saved");
        std::fs::remove_dir_all(directory).unwrap();
    }

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
