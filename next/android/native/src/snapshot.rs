//! Persist exactly the canonical accumulator, never its streaming overlay.
use crate::files::Files;
use misa_proto::sync::{ClientView, Version};
use misa_proto::{Node, Ticket};

pub fn name(ticket: &Ticket) -> String {
    let endpoint = ticket.node.split('@').next().unwrap_or(&ticket.node);
    format!(
        "view-{}.json",
        blake3::hash(format!("{endpoint}/{}", ticket.session).as_bytes()).to_hex()
    )
}

pub fn load(files: &Files, name: &str) -> Result<ClientView, String> {
    match std::fs::read(files.root.join(name)) {
        Ok(bytes) => {
            let (version, tree): (Version, Node) =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            ClientView::restore(version, tree)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ClientView::default()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn save(files: &Files, name: &str, client: &ClientView) -> Result<(), String> {
    if let Some(snapshot) = client.persisted() {
        files.write(
            name,
            &serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::SessionMsg;
    use misa_proto::sync::Stream;
    #[test]
    fn restart_restores_version_and_canonical_tree_without_streaming_text() {
        let files = Files::new(std::env::temp_dir().join(format!(
            "misa-snapshot-{}",
            iroh::SecretKey::generate().public()
        )))
        .unwrap();
        let version = Version {
            epoch: "daemon-epoch".into(),
            rev: 42,
        };
        let tree = Node::section("root").id("root");
        let mut client = ClientView::restore(version.clone(), tree.clone()).unwrap();
        client
            .receive(&SessionMsg::Streams {
                streams: vec![Stream {
                    id: "inflight.text".into(),
                    role: "message.assistant".into(),
                    text: "unfinished text".into(),
                }],
            })
            .unwrap();
        save(&files, "view.json", &client).unwrap();
        let loaded = load(&files, "view.json").unwrap();
        assert_eq!(loaded.version(), Some(&version));
        assert_eq!(loaded.canonical(), Some(tree));
        assert!(loaded.streams().is_empty());
        assert!(
            !std::fs::read_to_string(files.root.join("view.json"))
                .unwrap()
                .contains("unfinished text")
        );
        std::fs::remove_dir_all(files.root).unwrap();
    }
    #[test]
    fn snapshot_identity_is_session_scoped_and_survives_address_changes() {
        let first = Ticket {
            node: "endpoint@127.0.0.1:123".into(),
            session: "one".into(),
        };
        let moved = Ticket {
            node: "endpoint@127.0.0.1:456".into(),
            session: "one".into(),
        };
        let other = Ticket {
            node: first.node.clone(),
            session: "two".into(),
        };
        assert_eq!(name(&first), name(&moved));
        assert_ne!(name(&first), name(&other));
    }
}
