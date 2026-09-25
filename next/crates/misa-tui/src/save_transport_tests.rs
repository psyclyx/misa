#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::Session as _;
    use crate::save::{Request, target};
    #[tokio::test]
    async fn terminal_save_round_trips_session_and_blob_protocol_and_keeps_local_file() {
        let kernel = std::sync::Arc::new(misa_kernel::LocalKernel::new(
            misa_kernel::ScriptedProvider::always("done"),
        ));
        let blobs = kernel.blobs().clone();
        let stored = blobs
            .put(b"received attachment", Some("text/plain"))
            .unwrap();
        let runtime = misa_session::Runtime::start(
            "save",
            "Save",
            None,
            kernel,
            "scripted",
            "test",
            misa_value::Value::Null,
        );
        use misa_protocol::invocation::{CallContext, CommandOwner};
        let outcome = runtime
            .execute(
                &CallContext {
                    principal: "save-test".into(),
                    connection: 1,
                },
                misa_proto::invocation::Invocation {
                    id: 1,
                    scope: runtime.scope(),
                    command: "session.prompt".into(),
                    input: misa_value::Value::map([
                        ("text", misa_value::Value::str("file")),
                        (
                            "attachments",
                            misa_value::Value::list([misa_value::Value::map([
                                ("hash", misa_value::Value::str(&stored.hash)),
                                ("len", misa_value::Value::Int(stored.len as i64)),
                                (
                                    "media",
                                    stored
                                        .media
                                        .as_deref()
                                        .map(misa_value::Value::str)
                                        .unwrap_or(misa_value::Value::Null),
                                ),
                            ])]),
                        ),
                    ]),
                },
            )
            .await;
        assert!(matches!(
            outcome,
            misa_proto::invocation::Outcome::Accepted { .. }
        ));
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let directory = misa_daemon::directory::Directory::new("save-daemon").unwrap();
        directory.insert(runtime).unwrap();
        let admission = std::sync::Arc::new(misa_transport::admission::Admission::open());
        let router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(
                misa_proto::scoped::ALPN,
                misa_transport::scoped_server::Handler::new(
                    endpoint.id().to_string(),
                    directory.scope(),
                    std::sync::Arc::new(misa_daemon::directory::Routes(directory)),
                    admission.clone(),
                ),
            )
            .accept(
                misa_proto::ALPN_BLOB,
                misa_transport::blob::Handler {
                    blobs: std::sync::Arc::new(KernelBlobs(blobs)),
                    admission,
                },
            )
            .spawn();
        let client_endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let daemons = misa_client::daemons::Daemons::new(
            client_endpoint.clone(),
            misa_proto::ClientInfo::new("save-test", "1"),
        );
        let daemon = daemons
            .connect(
                misa_transport::iroh::address_of(&misa_transport::iroh::node_of(&endpoint))
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut updates = daemon.watch();
        let entry = loop {
            if let Some(entry) = daemon.sessions().unwrap().sessions.into_iter().next() {
                break entry;
            }
            updates.changed().await.unwrap();
        };
        let mut client = crate::scoped_remote::ScopedRemote::on(daemon, entry)
            .await
            .unwrap();
        let view = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(crate::Presentation::Documents(documents)) =
                    client.next_presentation().await.unwrap()
                {
                    if let Some(tree) =
                        documents.into_iter().find_map(|(id, update)| match update {
                            misa_client::document::Update::Reset(document) if id.is_empty() => {
                                Some(document.tree)
                            }
                            _ => None,
                        })
                    {
                        break tree;
                    }
                }
            }
        })
        .await
        .unwrap();
        let destination = std::env::temp_dir().join(format!(
            "misa-download-{}-{}.txt",
            std::process::id(),
            crate::test_unique_id()
        ));
        let request = Request {
            number: None,
            destination: destination.to_str().unwrap().into(),
        };
        assert!(
            client
                .save_attachment("missing-node", &request.destination)
                .await
                .is_err()
        );
        assert!(!destination.exists());
        let node = target(&view, &request).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.save_attachment(node, &request.destination),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"received attachment");
        assert!(
            client
                .save_attachment(node, &request.destination)
                .await
                .is_err()
        );
        std::fs::remove_file(destination).unwrap();
        router.shutdown().await.unwrap();
        endpoint.close().await;
        client_endpoint.close().await;
    }
}

#[cfg(test)]
struct KernelBlobs(std::sync::Arc<misa_kernel::Blobs>);
#[cfg(test)]
impl misa_transport::blob::BlobStore for KernelBlobs {
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
