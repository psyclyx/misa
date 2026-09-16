//! Same-user discovery and pairing through a private local socket.
use crate::admission::Admission;
use std::{
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub fn directory() -> Result<PathBuf, String> {
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or("Neither XDG_RUNTIME_DIR nor HOME is set")?;
    Ok(resolve_directory(root.join("misa-daemons")))
}

fn uid() -> u32 {
    // geteuid has no preconditions and does not mutate process state.
    unsafe { libc::geteuid() }
}

fn resolve_directory(requested: PathBuf) -> PathBuf {
    // BSD/macOS sun_path is 104 bytes, including the terminating NUL. Keep the
    // same conservative bound on all Unix targets. Never trust TMPDIR to be short.
    if socket_path(&requested, &"0".repeat(64))
        .as_os_str()
        .as_bytes()
        .len()
        < 104
    {
        return requested;
    }
    let hash = blake3::hash(requested.as_os_str().as_bytes()).to_hex();
    PathBuf::from(format!("/tmp/misa-{}-{}", uid(), &hash[..16]))
}

fn private_directory(dir: &Path) -> Result<bool, String> {
    let metadata = match std::fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    if !owned_private_directory(&metadata, uid()) {
        return Err(
            "Local daemon directory must be owned by this user and private (mode 0700)".into(),
        );
    }
    Ok(true)
}

fn owned_private_directory(metadata: &std::fs::Metadata, owner: u32) -> bool {
    metadata.is_dir() && metadata.uid() == owner && metadata.permissions().mode() & 0o777 == 0o700
}

fn same_user(stream: &tokio::net::UnixStream) -> Result<(), String> {
    if stream.peer_cred().map_err(|error| error.to_string())?.uid() != uid() {
        return Err("Local daemon socket belongs to another user".into());
    }
    Ok(())
}

pub struct Advertisement {
    path: PathBuf,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Advertisement {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn advertise(
    node: &str,
    session: &str,
    admission: Arc<Admission>,
) -> Result<Advertisement, String> {
    advertise_in(directory()?, node, session, admission)
}

fn advertise_in(
    dir: PathBuf,
    node: &str,
    session: &str,
    admission: Arc<Admission>,
) -> Result<Advertisement, String> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|e| e.to_string())?;
    private_directory(&dir)?;
    let id = node.split('@').next().unwrap_or(node);
    let path = socket_path(&dir, id);
    let listener = tokio::net::UnixListener::bind(&path).map_err(|e| e.to_string())?;
    let ticket = misa_proto::Ticket::new(node, session);
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            if same_user(&stream).is_err() {
                continue;
            }
            let handle = async {
                stream
                    .write_all(format!("{ticket}\n").as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
                let mut peer = String::new();
                BufReader::new((&mut stream).take(128))
                    .read_line(&mut peer)
                    .await
                    .map_err(|e| e.to_string())?;
                if peer.is_empty() {
                    return Ok::<(), String>(());
                }
                let peer: iroh::EndpointId = peer
                    .trim()
                    .parse()
                    .map_err(|_| "Invalid local client identity")?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                admission.admit_local(&peer.to_string(), now)?;
                stream.write_all(b"ok\n").await.map_err(|e| e.to_string())?;
                Ok(())
            };
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        }
    });
    Ok(Advertisement { path, task })
}

/// Stale sockets are ignored. Every live daemon gets its own entry.
pub async fn discover() -> Result<Vec<String>, String> {
    discover_in(directory()?).await
}
async fn discover_in(dir: PathBuf) -> Result<Vec<String>, String> {
    if !private_directory(&dir)? {
        return Ok(vec![]);
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.to_string()),
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "sock"))
        .collect();
    paths.sort();
    let mut found = vec![];
    for path in paths {
        let read = async {
            let stream = tokio::net::UnixStream::connect(path).await?;
            same_user(&stream).map_err(std::io::Error::other)?;
            let mut text = String::new();
            BufReader::new(stream.take(4096))
                .read_line(&mut text)
                .await?;
            Ok::<_, std::io::Error>(text.trim().to_string())
        };
        if let Ok(Ok(text)) =
            tokio::time::timeout(std::time::Duration::from_millis(500), read).await
        {
            if misa_proto::Pairing::given(&text).is_ok() {
                found.push(text);
            }
        }
    }
    Ok(found)
}

pub async fn pair(node: &str, peer: &str) -> Result<bool, String> {
    pair_in(directory()?, node, peer).await
}
async fn pair_in(dir: PathBuf, node: &str, peer: &str) -> Result<bool, String> {
    if !private_directory(&dir)? {
        return Ok(false);
    }
    let id = node.split('@').next().unwrap_or(node);
    // Parse before using an externally supplied name as a filename.
    let _: iroh::EndpointId = id.parse().map_err(|_| "Invalid daemon identity")?;
    let path = socket_path(&dir, id);
    let mut stream = match tokio::net::UnixStream::connect(path).await {
        Ok(stream) => stream,
        Err(_) => return Ok(false),
    };
    same_user(&stream)?;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut reader = BufReader::new(&mut stream);
        let mut ticket = String::new();
        reader
            .read_line(&mut ticket)
            .await
            .map_err(|e| e.to_string())?;
        let (ticket, _) = misa_proto::Pairing::given(ticket.trim())?;
        if ticket.node.split('@').next() != Some(id) {
            return Err("Local daemon identity mismatch".into());
        }
        reader
            .get_mut()
            .write_all(format!("{peer}\n").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut reply = String::new();
        reader
            .read_line(&mut reply)
            .await
            .map_err(|e| e.to_string())?;
        if reply.trim() != "ok" {
            return Err("Local pairing failed".into());
        }
        Ok(true)
    })
    .await
    .map_err(|_| "Local pairing timed out")?
}

fn socket_path(dir: &std::path::Path, id: &str) -> PathBuf {
    // Unix socket paths are small; authenticate the complete id in the greeting.
    dir.join(format!("{}.sock", id.chars().take(32).collect::<String>()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unique_dir() -> PathBuf {
        PathBuf::from("/tmp").join(format!(
            "misa-local-test-{}",
            iroh::SecretKey::generate().public().to_string()[..16].to_string()
        ))
    }
    #[tokio::test]
    async fn long_runtime_paths_share_a_private_short_namespace() {
        let requested = unique_dir()
            .join("long-runtime-directory".repeat(12))
            .join("misa-daemons");
        let dir = resolve_directory(requested.clone());
        assert_eq!(dir, resolve_directory(requested.clone()));
        assert_ne!(dir, resolve_directory(requested.join("different")));
        assert!(
            socket_path(&dir, &"0".repeat(64))
                .as_os_str()
                .as_bytes()
                .len()
                < 104
        );
        assert!(!requested.exists());
        let node = iroh::SecretKey::generate().public().to_string();
        let admission = Arc::new(Admission::paired(crate::admission::Paired::in_memory()));
        let registration = advertise_in(dir.clone(), &node, "long", admission.clone()).unwrap();
        assert!(private_directory(&dir).unwrap());
        assert_eq!(
            discover_in(dir.clone()).await.unwrap(),
            [misa_proto::Ticket::new(&node, "long").to_string()]
        );
        let peer = iroh::SecretKey::generate().public().to_string();
        assert!(pair_in(dir.clone(), &node, &peer).await.unwrap());
        assert!(admission.admits(&peer));
        drop(registration);
        assert!(discover_in(dir.clone()).await.unwrap().is_empty());
        std::fs::remove_dir(dir).unwrap();
    }
    #[tokio::test]
    async fn discovery_and_pairing_reject_public_and_symlink_directories() {
        let dir = unique_dir();
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(discover_in(dir.clone()).await.is_err());
        assert!(pair_in(dir.clone(), "unused", "unused").await.is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let metadata = std::fs::symlink_metadata(&dir).unwrap();
        assert!(owned_private_directory(&metadata, uid()));
        assert!(!owned_private_directory(&metadata, uid().wrapping_add(1)));
        let link = unique_dir();
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(discover_in(link.clone()).await.is_err());
        assert!(pair_in(link.clone(), "unused", "unused").await.is_err());
        std::fs::remove_file(link).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[tokio::test]
    async fn pairing_checks_complete_identity_before_sending_client_key() {
        let dir = unique_dir();
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let expected = iroh::SecretKey::generate().public().to_string();
        let wrong = iroh::SecretKey::generate().public().to_string();
        let path = socket_path(&dir, &expected);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream
                .write_all(format!("{}\n", misa_proto::Ticket::new(wrong, "wrong")).as_bytes())
                .await
                .unwrap();
            let mut bytes = [0; 128];
            assert_eq!(stream.read(&mut bytes).await.unwrap(), 0);
        });
        assert!(
            pair_in(dir.clone(), &expected, "never-send-this")
                .await
                .unwrap_err()
                .contains("identity mismatch")
        );
        task.await.unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[tokio::test]
    async fn discovery_pairs_multiple_local_clients_without_spending_a_remote_invitation() {
        let dir = std::env::temp_dir().join(format!("ml-{}", std::process::id()));
        let endpoint = crate::iroh::bind(None, false).await.unwrap();
        let node = crate::iroh::node_of(&endpoint);
        let admission = Arc::new(Admission::paired(crate::admission::Paired::in_memory()));
        let invitation = admission.invite(30_000, 100);
        let registration = advertise_in(dir.clone(), &node, "demo", admission.clone()).unwrap();
        assert_eq!(
            discover_in(dir.clone()).await.unwrap(),
            [misa_proto::Ticket::new(&node, "demo").to_string()]
        );
        let one = iroh::SecretKey::generate().public().to_string();
        let two = iroh::SecretKey::generate().public().to_string();
        assert!(pair_in(dir.clone(), &node, &one).await.unwrap());
        assert!(pair_in(dir.clone(), &node, &two).await.unwrap());
        assert!(admission.admits(&one) && admission.admits(&two));
        assert!(
            admission
                .accept("remote", invitation.code(), "remote", 101)
                .is_ok()
        );
        drop(registration);
        assert!(discover_in(dir.clone()).await.unwrap().is_empty());
        std::fs::remove_dir(dir).unwrap();
        endpoint.close().await;
    }
}
