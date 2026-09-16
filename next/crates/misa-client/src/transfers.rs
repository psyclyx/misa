//! Bounded asynchronous blob work shared by a daemon's presentation instances.
//! Returned bytes and decoded caches belong to the caller; admission bounds work
//! in flight, including blocking decoders after their awaiting future is dropped.
use misa_proto::{blob::MAX_BLOB_BYTES, view::BlobRef};
use misa_transport::blob::{Blob, Store};
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

#[derive(Clone, Copy)]
pub struct Limits {
    pub concurrent: usize,
    /// Conservative encoded-plus-decoded transfer buffers, not renderer caches.
    pub bytes: usize,
    pub timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            concurrent: 4,
            bytes: MAX_BLOB_BYTES * 4,
            timeout: Duration::from_secs(20),
        }
    }
}
struct Admission {
    _slot: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}
pub struct Transfers {
    store: Arc<Store>,
    limits: Limits,
    slots: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    closed: watch::Sender<bool>,
}
impl Transfers {
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        Self::with_limits(store, Limits::default()).expect("valid default transfer limits")
    }
    pub fn with_limits(store: Arc<Store>, limits: Limits) -> Result<Arc<Self>, String> {
        if limits.concurrent == 0
            || limits.concurrent > Semaphore::MAX_PERMITS
            || limits.bytes == 0
            || limits.bytes > u32::MAX as usize
            || limits.bytes > Semaphore::MAX_PERMITS
            || limits.timeout.is_zero()
        {
            return Err("Transfer limits must be nonzero and fit the byte budget".into());
        }
        Ok(Arc::new(Self {
            store,
            limits,
            slots: Arc::new(Semaphore::new(limits.concurrent)),
            bytes: Arc::new(Semaphore::new(limits.bytes)),
            closed: watch::channel(false).0,
        }))
    }
    fn admit(&self, bytes: usize) -> Result<Admission, String> {
        if *self.closed.borrow() {
            return Err("Daemon transfers are disconnected".into());
        }
        let bytes = u32::try_from(bytes.max(1)).map_err(|_| "Transfer exceeds the byte budget")?;
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "Too many active transfers; try again shortly")?;
        let bytes = self
            .bytes
            .clone()
            .try_acquire_many_owned(bytes)
            .map_err(|_| "Active transfers exceed the byte budget; try again shortly")?;
        Ok(Admission {
            _slot: slot,
            _bytes: bytes,
        })
    }
    async fn fetch(&self, hash: &str) -> Result<Option<Blob>, String> {
        self.run(self.store.get(hash), "Blob download timed out")
            .await
    }
    /// Explicit relationship removal cancels socket work and refuses new work.
    /// Already running blocking decoders finish while retaining their permits.
    pub fn close(&self) {
        self.closed.send_replace(true);
        self.slots.close();
        self.bytes.close();
    }
    async fn run<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, String>>,
        timeout: &str,
    ) -> Result<T, String> {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return Err("Daemon transfers are disconnected".into());
        }
        tokio::select! {
            _ = closed.changed() => Err("Daemon transfers are disconnected".into()),
            result = tokio::time::timeout(self.limits.timeout, work) => result.map_err(|_| timeout.to_string())?,
        }
    }
    pub async fn get(&self, hash: &str) -> Result<Option<Blob>, String> {
        // A peer's actual response is bounded by framing, not the offered length.
        let _admission = self.admit(MAX_BLOB_BYTES * 2)?;
        self.fetch(hash).await
    }
    pub async fn download(&self, reference: &BlobRef) -> Result<Blob, String> {
        let _admission = self.admit(MAX_BLOB_BYTES * 2)?;
        self.fetch_reference(reference).await
    }
    async fn fetch_reference(&self, reference: &BlobRef) -> Result<Blob, String> {
        if reference.len > MAX_BLOB_BYTES as u64 {
            return Err("Offered blob exceeds the transfer limit".into());
        }
        let blob = self
            .fetch(&reference.hash)
            .await?
            .ok_or("This attachment is no longer available")?;
        if blob.bytes.len() as u64 != reference.len {
            return Err("Attachment length does not match its reference".into());
        }
        Ok(blob)
    }
    pub async fn share(&self, bytes: Vec<u8>, media: Option<&str>) -> Result<BlobRef, String> {
        if bytes.len() > MAX_BLOB_BYTES {
            return Err("Attachment exceeds the transfer limit".into());
        }
        let _admission = self.admit(bytes.len().saturating_mul(2))?;
        self.run(
            self.store.share(bytes, media),
            "Blob upload timed out; content may already be stored",
        )
        .await
    }
    /// The decoder owns the admission lease until it finishes, even if its caller
    /// cancels. It must impose its own decoded-output limits (e.g. image pixels).
    pub async fn decode<T: Send + 'static>(
        &self,
        reference: &BlobRef,
        decode: impl FnOnce(&Blob) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let admission = self.admit(MAX_BLOB_BYTES * 2)?;
        let blob = self.fetch_reference(reference).await?;
        tokio::task::spawn_blocking(move || {
            let _admission = admission;
            decode(&blob)
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_transport::blob::BlobStore;
    const EMPTY: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    struct EmptyBlob;
    impl BlobStore for EmptyBlob {
        fn get(&self, hash: &str) -> Option<Vec<u8>> {
            (hash == EMPTY).then(Vec::new)
        }
        fn media(&self, _: &str) -> Option<String> {
            None
        }
        fn has(&self, hash: &str) -> bool {
            hash == EMPTY
        }
        fn store(&self, _: Vec<u8>, _: Option<&str>) -> Result<BlobRef, String> {
            Err("read only fixture".into())
        }
    }
    #[tokio::test]
    async fn cancelled_decoder_holds_its_budget_until_blocking_work_finishes() {
        let server = misa_transport::iroh::bind(None, false).await.unwrap();
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(
                misa_proto::ALPN_BLOB,
                misa_transport::blob::Handler {
                    blobs: Arc::new(EmptyBlob),
                    admission: Arc::new(misa_transport::admission::Admission::open()),
                },
            )
            .spawn();
        let address =
            misa_transport::iroh::address_of(&misa_transport::iroh::node_of(&server)).unwrap();
        let transfers = Transfers::with_limits(
            Store::new(endpoint.clone(), address),
            Limits {
                concurrent: 1,
                ..Limits::default()
            },
        )
        .unwrap();
        let reference = BlobRef {
            hash: EMPTY.into(),
            len: 0,
            media: None,
        };
        assert!(
            transfers
                .download(&BlobRef {
                    len: 1,
                    ..reference.clone()
                })
                .await
                .unwrap_err()
                .contains("length")
        );
        assert_eq!(transfers.share(vec![], None).await.unwrap().hash, EMPTY);
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let blocking_gate = gate.clone();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let work = transfers.clone();
        let decoding = tokio::spawn(async move {
            work.decode(&reference, move |blob| {
                assert!(blob.bytes.is_empty());
                let _ = entered.send(());
                let (lock, changed) = &*blocking_gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
                Ok(())
            })
            .await
        });
        ready.await.unwrap();
        decoding.abort();
        let _ = decoding.await;
        assert_eq!(transfers.slots.available_permits(), 0);
        assert!(
            transfers
                .get(EMPTY)
                .await
                .unwrap_err()
                .contains("active transfers")
        );
        {
            let (lock, changed) = &*gate;
            *lock.lock().unwrap() = true;
            changed.notify_one();
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while transfers.slots.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(transfers.get(EMPTY).await.unwrap().is_some());
        router.shutdown().await.unwrap();
        endpoint.close().await;
    }
    #[derive(Debug)]
    struct Silent;
    impl iroh::protocol::ProtocolHandler for Silent {
        async fn accept(
            &self,
            connection: iroh::endpoint::Connection,
        ) -> Result<(), iroh::protocol::AcceptError> {
            connection.closed().await;
            Ok(())
        }
    }
    #[tokio::test]
    async fn timed_out_transfer_releases_admission_and_discards_its_partial_socket() {
        let server = misa_transport::iroh::bind(None, false).await.unwrap();
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(misa_proto::ALPN_BLOB, Silent)
            .spawn();
        let address =
            misa_transport::iroh::address_of(&misa_transport::iroh::node_of(&server)).unwrap();
        let transfers = Transfers::with_limits(
            Store::new(endpoint.clone(), address),
            Limits {
                concurrent: 1,
                timeout: Duration::from_millis(100),
                ..Limits::default()
            },
        )
        .unwrap();
        for _ in 0..2 {
            assert!(
                transfers
                    .get(EMPTY)
                    .await
                    .unwrap_err()
                    .contains("timed out")
            );
            assert_eq!(transfers.slots.available_permits(), 1);
            assert_eq!(transfers.bytes.available_permits(), Limits::default().bytes);
        }
        let fetching = transfers.clone();
        let pending = tokio::spawn(async move { fetching.get(EMPTY).await });
        while transfers.slots.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
        transfers.close();
        assert!(pending.await.unwrap().unwrap_err().contains("disconnected"));
        assert!(
            transfers
                .get(EMPTY)
                .await
                .unwrap_err()
                .contains("disconnected")
        );
        router.shutdown().await.unwrap();
        endpoint.close().await;
    }
}
