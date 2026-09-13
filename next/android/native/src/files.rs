//! App-private storage. The platform supplies the directory; no home-directory discovery.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub struct Files {
    pub root: PathBuf,
}
impl Files {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        std::fs::create_dir_all(root.join("blobs")).map_err(|e| e.to_string())?;
        Ok(Self { root })
    }
    pub fn identity(&self) -> Result<iroh::SecretKey, String> {
        static INITIALIZING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = INITIALIZING
            .lock()
            .map_err(|_| "identity storage lock is poisoned")?;
        let path = self.root.join("identity");
        match std::fs::read(&path) {
            Ok(bytes) => {
                let bytes: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| "invalid saved endpoint identity")?;
                Ok(iroh::SecretKey::from_bytes(&bytes))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let secret = iroh::SecretKey::generate();
                self.write("identity", &secret.to_bytes())?;
                Ok(secret)
            }
            Err(error) => Err(error.to_string()),
        }
    }
    pub fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.root.join(name);
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let temporary = path.with_extension(format!(
            "{}.tmp",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        std::fs::rename(temporary, path).map_err(|e| e.to_string())
    }
    pub fn blob_path(&self, hash: &str) -> Result<PathBuf, String> {
        if !misa_proto::blob::valid_hash(hash) {
            return Err("invalid blob hash".into());
        }
        Ok(self.root.join("blobs").join(hash))
    }
    pub fn cache(&self, hash: &str, bytes: &[u8]) -> Result<PathBuf, String> {
        let path = self.blob_path(hash)?;
        if blake3::hash(bytes).to_hex().as_str() != hash {
            return Err("blob content hash mismatch".into());
        }
        self.write(&format!("blobs/{hash}"), bytes)?;
        Ok(path)
    }
    pub fn cached(&self, hash: &str) -> Result<Option<PathBuf>, String> {
        let path = self.blob_path(hash)?;
        match std::fs::read(&path) {
            Ok(bytes) if blake3::hash(&bytes).to_hex().as_str() == hash => Ok(Some(path)),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}

pub struct UploadFile(pub PathBuf);
impl Drop for UploadFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn upload_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(misa_proto::blob::MAX_BLOB_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > misa_proto::blob::MAX_BLOB_BYTES {
        return Err("attachments are limited to 32 MiB".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn files() -> Files {
        Files::new(std::env::temp_dir().join(format!(
            "misa-android-{}",
            iroh::SecretKey::generate().public()
        )))
        .unwrap()
    }
    #[test]
    fn identity_survives_restart_and_corruption_is_not_silently_replaced() {
        let files = files();
        let first = files.identity().unwrap();
        assert_eq!(files.identity().unwrap().public(), first.public());
        files.write("identity", b"broken").unwrap();
        assert!(files.identity().is_err());
        std::fs::remove_dir_all(files.root).unwrap();
    }
    #[test]
    fn cached_bytes_are_verified_and_a_hash_cannot_escape_storage() {
        let files = files();
        let bytes = b"picture";
        let hash = blake3::hash(bytes).to_hex().to_string();
        assert!(files.blob_path("../../secret").is_err());
        assert!(files.cache(&hash, b"different").is_err());
        let path = files.cache(&hash, bytes).unwrap();
        assert_eq!(files.cached(&hash).unwrap(), Some(path.clone()));
        std::fs::write(path, b"corrupt").unwrap();
        assert!(files.cached(&hash).unwrap().is_none());
        std::fs::remove_dir_all(files.root).unwrap();
    }
    #[test]
    fn uploads_are_bounded_before_entering_the_transport() {
        let files = files();
        let path = files.root.join("upload");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(misa_proto::blob::MAX_BLOB_BYTES as u64 + 1)
            .unwrap();
        assert!(upload_bytes(&path).is_err());
        std::fs::write(&path, b"hello").unwrap();
        assert_eq!(upload_bytes(&path).unwrap(), b"hello");
        std::fs::remove_dir_all(files.root).unwrap();
    }
}
