//! Content-addressed bytes.
//!
//! An image, an attachment, a tool's large output: anything too big or too binary to
//! inline in a message. A `BlobRef` names it by hash, and whoever has it serves it.
//! The previous system kept images as original bytes for provider requests and a
//! bounded preview for drawing, which is the same split: the bytes are the truth and
//! the preview is a presentation of them.
//!
//! Content addressing is what makes this safe to serve to any client: a hash is not a
//! path, so a client cannot ask for `/etc/shadow` by lying about a filename.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use misa_proto::view::BlobRef;

/// The largest thing that will be stored.
///
/// The same number the wire uses, and deliberately not a second one: a blob the store
/// accepted and the protocol refused would be a file that exists and can never be served.
/// It is `usize` here because a size is a size; the protocol's `MAX_BLOB_BYTES` is what
/// that number comes from.
pub const MAX_BLOB: usize = misa_proto::MAX_BLOB_BYTES;

/// How much the store will hold in total.
///
/// A per-blob bound is not a bound on a store: a peer that is allowed to upload can send
/// 32 MiB at a time, forever, and fill a disk. This is the other half of the same bound,
/// and it is enforced where the bytes are written rather than where they arrive, so a
/// tool that reads a huge file is refused here too.
pub const MAX_STORE: u64 = 512 * 1024 * 1024;

/// Why bytes were refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// One blob larger than [`MAX_BLOB`].
    TooLarge { len: usize, max: usize },
    /// The store as a whole is full.
    Full { held: u64, adding: u64, max: u64 },
}

/// Whether bytes of this size may be added.
///
/// `held` is what the store holds now and `adding` is how much of it would be new, which
/// is not the same as the size of the blob: storing bytes the store already has is free.
/// Split out from [`Blobs::put`] so the bounds can be checked without filling a disk to
/// reach them.
pub fn check(len: usize, held: u64, adding: u64) -> Result<(), Refused> {
    if len > MAX_BLOB {
        return Err(Refused::TooLarge { len, max: MAX_BLOB });
    }
    if held + adding > MAX_STORE {
        return Err(Refused::Full { held, adding, max: MAX_STORE });
    }
    Ok(())
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::TooLarge { len, max } => write!(f, "{len} bytes is larger than the {max} byte bound"),
            Refused::Full { held, adding, max } => write!(
                f,
                "the blob store holds {held} of {max} bytes; {adding} more would not fit"
            ),
        }
    }
}

pub struct Blobs {
    /// `None` keeps everything in memory, which is what a test and a session with no
    /// data directory get.
    root: Option<PathBuf>,
    index: std::sync::Mutex<HashMap<String, (u64, Option<String>)>>,
    /// Bytes, when there is no directory to hold them.
    memory: std::sync::Mutex<HashMap<String, Vec<u8>>>,
}

impl Blobs {
    pub fn in_memory() -> Blobs {
        Blobs {
            root: None,
            index: std::sync::Mutex::new(HashMap::new()),
            memory: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn at(root: &Path) -> Result<Blobs, String> {
        std::fs::create_dir_all(root).map_err(|err| err.to_string())?;
        let mut index = HashMap::new();
        // The directory *is* the index: a hash is a filename, so a restart finds what it
        // wrote without a second thing to keep in step.
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(hash) = name.strip_suffix(".bin")
                    && let Ok(metadata) = entry.metadata()
                    && valid_hash(hash)
                {
                    index.insert(hash.to_string(), (metadata.len(), None));
                }
            }
        }
        Ok(Blobs {
            root: Some(root.to_path_buf()),
            index: std::sync::Mutex::new(index),
            memory: std::sync::Mutex::new(HashMap::new()),
        })
    }

    pub fn put(&self, bytes: &[u8], media: Option<&str>) -> Result<BlobRef, String> {
        self.store(bytes.to_vec(), media)
    }

    /// Store bytes the caller already owns.
    ///
    /// What a network upload has: the bytes arrived as a `Vec` and copying them again to
    /// hand them over would double the memory a 32 MiB attachment costs for no reason.
    pub fn store(&self, bytes: Vec<u8>, media: Option<&str>) -> Result<BlobRef, String> {
        // Two bounds, checked here rather than where bytes arrive, against what would
        // actually be *added*: re-storing bytes the store already holds costs nothing and
        // is never refused for being full.
        let hash = hash_of(&bytes);
        let adding = if self.has(&hash) { 0 } else { bytes.len() as u64 };
        check(bytes.len(), self.bytes(), adding).map_err(|refused| refused.to_string())?;
        let media = media.map(str::to_string).or_else(|| sniff(&bytes).map(str::to_string));
        if let Some(root) = &self.root {
            let path = root.join(format!("{hash}.bin"));
            if !path.exists() {
                // Write beside and rename, so a reader never sees a partial blob.
                let temporary = root.join(format!("{hash}.part"));
                std::fs::write(&temporary, &bytes).map_err(|err| err.to_string())?;
                std::fs::rename(&temporary, &path).map_err(|err| err.to_string())?;
            }
            if let Some(media) = &media {
                let _ = std::fs::write(root.join(format!("{hash}.media")), media);
            }
        }
        let len = bytes.len() as u64;
        if self.root.is_none() {
            self.memory
                .lock()
                .map_err(|_| "the blob store is poisoned".to_string())?
                .insert(hash.clone(), bytes);
        }
        let mut index = self.index.lock().map_err(|_| "the blob index is poisoned".to_string())?;
        index.insert(hash.clone(), (len, media.clone()));
        Ok(BlobRef { hash, len, media })
    }

    /// Metadata for a stored, well-formed content name. No path crosses this boundary.
    pub fn describe(&self, hash: &str) -> Option<BlobRef> {
        if !valid_hash(hash) { return None; }
        let len = if let Some(root) = &self.root {
            std::fs::metadata(root.join(format!("{hash}.bin"))).ok()?.len()
        } else { self.index.lock().ok()?.get(hash)?.0 };
        Some(BlobRef { hash: hash.to_string(), len, media: self.media(hash) })
    }

    /// Read bytes back, from memory or from the store.
    pub fn get(&self, hash: &str) -> Option<Vec<u8>> {
        if !valid_hash(hash) {
            return None;
        }
        if let Some(root) = &self.root
            && let Ok(bytes) = std::fs::read(root.join(format!("{hash}.bin")))
        {
            return Some(bytes);
        }
        self.memory.lock().ok()?.get(hash).cloned()
    }

    /// Where a blob is on disk, for a server that would rather stream a file than
    /// copy it.
    pub fn path(&self, hash: &str) -> Option<PathBuf> {
        if !valid_hash(hash) {
            return None;
        }
        let path = self.root.as_ref()?.join(format!("{hash}.bin"));
        path.exists().then_some(path)
    }

    pub fn media(&self, hash: &str) -> Option<String> {
        if let Some(root) = &self.root
            && let Ok(media) = std::fs::read_to_string(root.join(format!("{hash}.media")))
        {
            let media = media.trim();
            if !media.is_empty() {
                return Some(media.to_string());
            }
        }
        self.index.lock().ok()?.get(hash).and_then(|(_, media)| media.clone())
    }

    pub fn has(&self, hash: &str) -> bool {
        if !valid_hash(hash) {
            return false;
        }
        self.index.lock().map(|index| index.contains_key(hash)).unwrap_or(false)
            || self.memory.lock().map(|memory| memory.contains_key(hash)).unwrap_or(false)
    }

    /// The total size of everything held, which is what the store's own bound is about.
    pub fn bytes(&self) -> u64 {
        self.index
            .lock()
            .map(|index| index.values().map(|(len, _)| *len).sum())
            .unwrap_or(0)
    }

    /// Every hash the store holds.
    pub fn hashes(&self) -> Vec<String> {
        let mut hashes: Vec<String> = self
            .index
            .lock()
            .map(|index| index.keys().cloned().collect())
            .unwrap_or_default();
        hashes.sort();
        hashes
    }

    pub fn len(&self) -> usize {
        self.index.lock().map(|index| index.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Blobs {
    fn default() -> Self {
        Blobs::in_memory()
    }
}

/// The content address of some bytes.
pub fn hash_of(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// A hash is 64 lowercase hex characters, and anything else is refused before it
/// becomes a filename.
///
/// The rule itself lives in [`misa_proto::blob`], because a client checks it too and two
/// copies of "what a hash looks like" is one copy too many.
pub fn valid_hash(hash: &str) -> bool {
    misa_proto::blob::valid_hash(hash)
}

/// The media type of some bytes, from the bytes.
///
/// Sniffed rather than trusted: a client sends a filename, a file contains magic
/// numbers, and the magic numbers are the part a client cannot lie about cheaply.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        [0x25, b'P', b'D', b'F', ..] => Some("application/pdf"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];

    #[test]
    fn the_same_bytes_get_the_same_name_and_a_different_name_otherwise() {
        let blobs = Blobs::in_memory();
        let first = blobs.put(PNG, None).unwrap();
        let second = blobs.put(PNG, None).unwrap();
        assert_eq!(first.hash, second.hash);
        assert_eq!(first.len, PNG.len() as u64);
        let other = blobs.put(b"different", None).unwrap();
        assert_ne!(first.hash, other.hash);
        assert_eq!(blobs.len(), 2);
    }

    #[test]
    fn the_media_type_comes_from_the_bytes_rather_than_from_a_caller() {
        let blobs = Blobs::in_memory();
        assert_eq!(blobs.put(PNG, None).unwrap().media.as_deref(), Some("image/png"));
        assert_eq!(blobs.put(b"just text", None).unwrap().media, None);
        // A caller that knows better may say so.
        assert_eq!(
            blobs.put(b"just text", Some("text/plain")).unwrap().media.as_deref(),
            Some("text/plain")
        );
    }

    #[test]
    fn a_hash_that_is_not_a_hash_cannot_name_a_file() {
        let blobs = Blobs::in_memory();
        blobs.put(b"x", None).unwrap();
        assert!(blobs.get("../../etc/shadow").is_none());
        assert!(!blobs.has("../../etc/shadow"));
        assert!(blobs.get(&"A".repeat(64)).is_none(), "an uppercase hash is not one of ours");
        assert!(blobs.get(&"a".repeat(63)).is_none());
    }

    #[test]
    fn bytes_survive_a_restart_because_the_directory_is_the_index() {
        let root = std::env::temp_dir().join(format!("misa-blobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let stored = {
            let blobs = Blobs::at(&root).unwrap();
            blobs.put(PNG, None).unwrap()
        };
        let reopened = Blobs::at(&root).unwrap();
        assert!(reopened.has(&stored.hash), "a blob was lost across a reopen");
        assert_eq!(reopened.get(&stored.hash).as_deref(), Some(PNG));
        assert_eq!(reopened.media(&stored.hash).as_deref(), Some("image/png"));
        assert!(reopened.path(&stored.hash).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn something_larger_than_the_bound_is_refused() {
        let blobs = Blobs::in_memory();
        let huge = vec![0u8; MAX_BLOB + 1];
        assert!(blobs.put(&huge, None).is_err());
    }

    #[test]
    fn the_store_reports_what_it_holds() {
        let blobs = Blobs::in_memory();
        assert_eq!(blobs.bytes(), 0);
        assert!(blobs.is_empty());
        let first = blobs.put(PNG, None).unwrap();
        let second = blobs.put(b"more bytes", None).unwrap();
        assert_eq!(blobs.bytes(), (PNG.len() + "more bytes".len()) as u64);
        let mut expected = vec![first.hash.clone(), second.hash.clone()];
        expected.sort();
        assert_eq!(blobs.hashes(), expected);
        // Storing the same bytes twice adds nothing, which is the property that makes an
        // upload free to repeat.
        blobs.put(PNG, None).unwrap();
        assert_eq!(blobs.bytes(), (PNG.len() + "more bytes".len()) as u64);
        assert!(!blobs.is_empty());
    }

    #[test]
    fn the_store_holds_the_bounds_it_says_it_does() {
        // The bounds themselves, checked as arithmetic rather than by filling a disk to
        // reach them: this is the function `put` calls.
        assert!(matches!(check(MAX_BLOB + 1, 0, 0), Err(Refused::TooLarge { .. })));
        assert!(check(MAX_BLOB, 0, MAX_BLOB as u64).is_ok());
        assert!(matches!(check(1, MAX_STORE, 1), Err(Refused::Full { .. })));
        // Bytes the store already holds are not bytes it has to find room for, so a store
        // at its bound still accepts what it has.
        assert!(check(1, MAX_STORE, 0).is_ok());
        // A store with a byte of room takes a one-byte blob and not a two-byte one.
        assert!(check(1, MAX_STORE - 1, 1).is_ok());
        assert!(matches!(check(2, MAX_STORE - 1, 2), Err(Refused::Full { .. })));
        let message = Refused::Full { held: 10, adding: 10, max: MAX_STORE }.to_string();
        assert!(message.contains("would not fit"), "{message}");
    }
}
