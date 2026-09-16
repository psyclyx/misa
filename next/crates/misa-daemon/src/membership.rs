//! Desired open sessions survive daemon restart; runtime incarnations do not.
use crate::lifecycle::SessionSpec;
use misa_proto::Fault;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

pub trait MembershipStore: Send + Sync {
    fn load(&self) -> Result<Vec<SessionSpec>, Fault>;
    fn save(&self, sessions: &[SessionSpec]) -> Result<(), Fault>;
}
#[derive(Default)]
pub struct Memory(Mutex<Vec<SessionSpec>>);
impl MembershipStore for Memory {
    fn load(&self) -> Result<Vec<SessionSpec>, Fault> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, sessions: &[SessionSpec]) -> Result<(), Fault> {
        *self.0.lock().unwrap() = sessions.to_vec();
        Ok(())
    }
}
pub struct File {
    path: PathBuf,
}
impl File {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}
fn storage(error: impl std::fmt::Display) -> Fault {
    Fault::new("membership_storage", error.to_string())
}
pub(crate) fn write_atomic(path: &Path, value: &impl serde::Serialize) -> Result<(), Fault> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(storage)?;
    let temporary = path.with_extension("pending");
    let result = (|| {
        let mut file = std::fs::File::create(&temporary).map_err(storage)?;
        ciborium::ser::into_writer(value, &mut file).map_err(storage)?;
        file.sync_all().map_err(storage)?;
        std::fs::rename(&temporary, path).map_err(storage)?;
        std::fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(storage)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
impl MembershipStore for File {
    fn load(&self) -> Result<Vec<SessionSpec>, Fault> {
        match std::fs::File::open(&self.path) {
            Ok(file) => {
                if file.metadata().map_err(storage)?.len() > 1024 * 1024 {
                    return Err(storage("Desired session file exceeds limit"));
                }
                let records: Vec<SessionSpec> = ciborium::de::from_reader(file).map_err(storage)?;
                if records.len() > 256 {
                    return Err(storage("Desired session capacity exceeded"));
                }
                Ok(records)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(error) => Err(storage(error)),
        }
    }
    fn save(&self, sessions: &[SessionSpec]) -> Result<(), Fault> {
        write_atomic(&self.path, &sessions)
    }
}
