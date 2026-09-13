//! Connect the daemon's kernel store to the transport's storage interface.
use std::sync::Arc;
pub struct Store(pub Arc<misa_kernel::Blobs>);
impl misa_net::blob::BlobStore for Store {
    fn get(&self, hash: &str) -> Option<Vec<u8>> { self.0.get(hash) }
    fn media(&self, hash: &str) -> Option<String> { self.0.media(hash) }
    fn has(&self, hash: &str) -> bool { self.0.has(hash) }
    fn store(&self, bytes: Vec<u8>, media: Option<&str>) -> Result<misa_proto::view::BlobRef, String> { self.0.store(bytes, media) }
}
