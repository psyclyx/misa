//! Mount independent scoped application, pairing admission and blob protocols.
use crate::{admission::Admission, blob};
use iroh::{Endpoint, protocol::Router};
use std::sync::Arc;

pub fn serve(
    endpoint: Endpoint,
    blobs: Arc<dyn blob::BlobStore>,
    admission: Arc<Admission>,
    scoped: crate::scoped_server::Handler,
) -> Router {
    Router::builder(endpoint)
        .accept(misa_proto::scoped::ALPN, scoped)
        .accept(
            crate::pairing::ALPN,
            crate::pairing::Handler {
                admission: admission.clone(),
            },
        )
        .accept(misa_proto::ALPN_BLOB, blob::Handler { blobs, admission })
        .spawn()
}

#[cfg(test)]
pub(crate) struct Fixture {
    pub client: Endpoint,
    pub address: iroh::EndpointAddr,
    pub blobs: Arc<misa_kernel::Blobs>,
    pub router: Router,
}
#[cfg(test)]
impl Fixture {
    pub(crate) async fn start(
        admission: Admission,
        _provider: Arc<dyn misa_kernel::Provider>,
    ) -> Self {
        let server = crate::iroh::bind(None, false).await.unwrap();
        let client = crate::iroh::bind(None, false).await.unwrap();
        let address = crate::iroh::address_of(&crate::iroh::node_of(&server)).unwrap();
        let blobs = Arc::new(misa_kernel::Blobs::in_memory());
        let router = Router::builder(server)
            .accept(
                misa_proto::ALPN_BLOB,
                blob::Handler {
                    blobs: Arc::new(KernelBlobs(blobs.clone())),
                    admission: Arc::new(admission),
                },
            )
            .spawn();
        Self {
            client,
            address,
            blobs,
            router,
        }
    }
    pub(crate) async fn stop(self) {
        self.router.shutdown().await.unwrap();
        self.client.close().await;
    }
}
#[cfg(test)]
struct KernelBlobs(Arc<misa_kernel::Blobs>);
#[cfg(test)]
impl blob::BlobStore for KernelBlobs {
    fn get(&self, hash: &str) -> Option<Vec<u8>> {
        self.0.get(hash)
    }
    fn media(&self, hash: &str) -> Option<String> {
        self.0.media(hash)
    }
    fn has(&self, hash: &str) -> bool {
        self.0.has(hash)
    }
    fn store(
        &self,
        bytes: Vec<u8>,
        media: Option<&str>,
    ) -> Result<misa_proto::view::BlobRef, String> {
        self.0.store(bytes, media)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        Fault,
        observation::{Scope, ScopeId},
    };
    struct Empty;
    impl misa_protocol::owner::Resolver for Empty {
        fn resolve(
            &self,
            _: &misa_protocol::invocation::CallContext,
            _: &Scope,
        ) -> Result<Arc<dyn misa_protocol::owner::Owner>, Fault> {
            Err(Fault::new("scope_unavailable", "No session"))
        }
    }
    #[tokio::test]
    async fn pairing_admits_same_identity_to_scoped_and_blob_without_session_attach() {
        let server = crate::iroh::bind(None, false).await.unwrap();
        let client = crate::iroh::bind(None, false).await.unwrap();
        let address = crate::iroh::address_of(&crate::iroh::node_of(&server)).unwrap();
        let admission = Arc::new(Admission::listed([]));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let invitation = admission.invite(60_000, now);
        let scope = Scope {
            id: ScopeId::Daemon,
            incarnation: "daemon-test".into(),
        };
        let router = serve(
            server.clone(),
            Arc::new(KernelBlobs(Arc::new(misa_kernel::Blobs::in_memory()))),
            admission.clone(),
            crate::scoped_server::Handler {
                daemon: server.id().to_string(),
                scope: scope.clone(),
                resolver: Arc::new(Empty),
                admission: admission.clone(),
            },
        );
        let info = misa_proto::ClientInfo::new("test", "1");
        assert!(
            crate::scoped_client::Client::connect(&client, address.clone(), info.clone())
                .await
                .is_err()
        );
        assert!(
            client
                .connect(address.clone(), misa_proto::ALPN_SESSION)
                .await
                .is_err(),
            "removed Attach ALPN is still mounted"
        );
        crate::pairing::pair(&client, address.clone(), invitation.code(), "test")
            .await
            .unwrap();
        let scoped = crate::scoped_client::Client::connect(&client, address.clone(), info)
            .await
            .unwrap();
        assert_eq!(scoped.welcome.scope, scope);
        let mut blobs = blob::Client::connect(&client, address.clone())
            .await
            .unwrap();
        let stored = blobs
            .put(b"shared admission".to_vec(), Some("text/plain"))
            .await
            .unwrap();
        assert_eq!(
            blobs.get(&stored.hash).await.unwrap().unwrap().bytes,
            b"shared admission"
        );
        admission.revoke(&client.id().to_string()).unwrap();
        assert!(blobs.get(&stored.hash).await.is_err());
        drop(scoped);
        drop(blobs);
        router.shutdown().await.unwrap();
        client.close().await;
    }
}
