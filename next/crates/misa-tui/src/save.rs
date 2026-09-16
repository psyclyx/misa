//! A terminal destination is local state; only the selected attachment action crosses the wire.
use misa_proto::view::Node;

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub number: Option<usize>,
    pub destination: String,
}

pub fn parse(line: &str) -> Option<Result<Request, String>> {
    let rest = line.strip_prefix("/save")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return Some(Err(
            "Use /save [attachment number] <local path>; omit the number for the latest attachment"
                .into(),
        ));
    }
    let (number, destination) = match rest.split_once(char::is_whitespace) {
        Some((first, path)) if first.parse::<usize>().is_ok() => (first.parse().ok(), path.trim()),
        _ => (None, rest),
    };
    if number == Some(0) || destination.is_empty() {
        return Some(Err(
            "Attachment numbers start at 1; a local path is required".into(),
        ));
    }
    Some(Ok(Request {
        number,
        destination: destination.to_string(),
    }))
}

pub fn attachments(view: &Node) -> Vec<&Node> {
    fn visit<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
        if node
            .actions
            .iter()
            .any(|action| action.id == "attachment.save")
        {
            out.push(node);
        }
        for child in &node.children {
            visit(child, out);
        }
    }
    let mut nodes = vec![];
    visit(view, &mut nodes);
    nodes
}

pub fn target<'a>(view: &'a Node, request: &Request) -> Result<&'a str, String> {
    let attachments = attachments(view);
    let index = request.number.unwrap_or(attachments.len());
    attachments
        .get(index.saturating_sub(1))
        .map(|node| node.id.as_str())
        .ok_or_else(|| "No attachment with that number is in this view".into())
}

pub fn write_new(destination: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("Could not create {destination}: {error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Could not finish {destination}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_destinations_and_attachment_selection_are_not_commands_sent_to_session() {
        assert_eq!(
            parse("/save 2 /tmp/my file.png").unwrap().unwrap(),
            Request {
                number: Some(2),
                destination: "/tmp/my file.png".into()
            }
        );
        assert_eq!(parse("/save ./photo.png").unwrap().unwrap().number, None);
        assert!(parse("/save").unwrap().is_err());
        assert!(parse("/save 0 x").unwrap().is_err());
        assert!(parse("/saved").is_none());
    }
    #[test]
    fn saving_creates_an_exact_file_and_refuses_to_overwrite() {
        let destination = std::env::temp_dir().join(format!(
            "misa-save-{}-{}.bin",
            std::process::id(),
            crate::test_unique_id()
        ));
        let path = destination.to_str().unwrap();
        write_new(path, b"received bytes").unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"received bytes");
        assert!(write_new(path, b"replacement").is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"received bytes");
        std::fs::remove_file(destination).unwrap();
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::Session as _;
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
        let outcome = runtime.execute(&CallContext { principal: "save-test".into(), connection: 1 }, misa_proto::invocation::Invocation {
            id: 1, scope: runtime.scope(), command: "session.prompt".into(),
            input: misa_value::Value::map([
                ("text", misa_value::Value::str("file")),
                ("attachments", misa_value::Value::list([misa_value::Value::map([
                    ("hash", misa_value::Value::str(&stored.hash)),
                    ("len", misa_value::Value::Int(stored.len as i64)),
                    ("media", stored.media.as_deref().map(misa_value::Value::str).unwrap_or(misa_value::Value::Null)),
                ])])),
            ]),
        }).await;
        assert!(matches!(outcome, misa_proto::invocation::Outcome::Accepted { .. }));
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let directory = misa_daemon::directory::Directory::new("save-daemon").unwrap();
        directory.insert(runtime).unwrap();
        let admission = std::sync::Arc::new(misa_transport::admission::Admission::open());
        let router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(
                misa_proto::scoped::ALPN,
                misa_transport::scoped_server::Handler {
                    daemon: endpoint.id().to_string(),
                    scope: directory.scope(),
                    resolver: std::sync::Arc::new(misa_daemon::directory::Routes(directory)),
                    admission: admission.clone(),
                },
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
                if let Some(crate::Presentation::Documents(documents)) = client.next_presentation().await.unwrap() {
                    if let Some(tree)=documents.into_iter().find_map(|(id,update)|match update{misa_client::document::Update::Reset(document) if id.is_empty()=>Some(document.tree),_=>None}) {break tree;}
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
