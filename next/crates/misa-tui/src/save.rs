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
            "Use /save [attachment number] <local path>; omit the number for the latest attachment".into()
        ));
    }
    let (number, destination) = match rest.split_once(char::is_whitespace) {
        Some((first, path)) if first.parse::<usize>().is_ok() => (first.parse().ok(), path.trim()),
        _ => (None, rest),
    };
    if number == Some(0) || destination.is_empty() {
        return Some(Err("Attachment numbers start at 1; a local path is required".into()));
    }
    Some(Ok(Request { number, destination: destination.to_string() }))
}

pub fn attachments(view: &Node) -> Vec<&Node> {
    fn visit<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
        if node.actions.iter().any(|action| action.id == "attachment.save") {
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
            Request { number: Some(2), destination: "/tmp/my file.png".into() }
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
            misa_proto::wire::RequestContext::connection()
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
        let kernel = std::sync::Arc::new(misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::always("done")));
        let blobs = kernel.blobs().clone();
        let stored = blobs.put(b"received attachment", Some("text/plain")).unwrap();
        let runtime =
            misa_session::Runtime::start("save", "Save", None, kernel, "scripted", "test", misa_value::Value::Null);
        runtime.intent(misa_proto::wire::Intent::Prompt { text: "file".into(), attachments: vec![stored] });
        let endpoint = misa_net::iroh::bind(None, false).await.unwrap();
        let sessions = misa_net::iroh::Sessions::new();
        sessions.insert(runtime);
        let router = misa_net::server::serve(
            endpoint.clone(),
            sessions,
            blobs,
            std::sync::Arc::new(misa_net::admission::Admission::open()),
        );
        let ticket = misa_net::iroh::ticket(&endpoint, "save").to_string();
        let mut client = crate::Remote::attach(&ticket).await.unwrap();
        let view =
            tokio::time::timeout(std::time::Duration::from_secs(5), client.next()).await.unwrap().unwrap().unwrap();
        let destination = std::env::temp_dir().join(format!(
            "misa-download-{}-{}.txt",
            std::process::id(),
            misa_proto::wire::RequestContext::connection()
        ));
        let request = Request { number: None, destination: destination.to_str().unwrap().into() };
        let node = target(&view, &request).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), client.save_attachment(node, &request.destination))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"received attachment");
        assert!(client.save_attachment(node, &request.destination).await.is_err());
        std::fs::remove_file(destination).unwrap();
        router.shutdown().await.unwrap();
        endpoint.close().await;
    }
}
