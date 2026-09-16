//! Canonical-only scoped checkpoints, never live output or owner-neutral cursors.
use crate::files::Files;
use misa_proto::observation::{Content, Selection};
use misa_protocol::observation::Checkpoint;
use std::{io::Read, sync::Arc};
pub fn index_name(daemon: &str, scope: &misa_proto::observation::Scope) -> String {
    format!(
        "saved-{}.json",
        blake3::hash(&serde_json::to_vec(&(daemon, scope)).expect("scope serializes")).to_hex()
    )
}
pub async fn cached(files: Arc<Files>) -> Option<serde_json::Value> {
    let storage = files.clone();
    let metadata: serde_json::Value = tokio::task::spawn_blocking(move || {
        std::fs::read(storage.root.join("selected.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    })
    .await
    .ok()??;
    let daemon = metadata["daemon"].as_str()?;
    let scope: misa_proto::observation::Scope =
        serde_json::from_value(metadata["scope"].clone()).ok()?;
    let index = index_name(daemon, &scope);
    let storage = files.clone();
    let name =
        tokio::task::spawn_blocking(move || std::fs::read_to_string(storage.root.join(index)).ok())
            .await
            .ok()??;
    if !name.starts_with("view-")
        || !name.ends_with(".json")
        || name.contains('/')
        || name.contains('\\')
    {
        return None;
    }
    let checkpoint = load(files, name).await.ok()??;
    if checkpoint.resume.selection.scope != scope {
        return None;
    }
    let mut documents = serde_json::Map::new();
    for (id, content) in checkpoint.members {
        if let Content::Document(document) = content {
            misa_proto::view::validate(&document.tree).ok()?;
            documents.insert(
                id,
                serde_json::json!({"mode":"reset","tree":document.tree,"streams":[]}),
            );
        }
    }
    Some(
        serde_json::json!({"kind":"cached","daemon":daemon,"scope":scope,"title":metadata["title"],"documents":documents}),
    )
}
pub fn name(daemon: &str, selection: &Selection) -> String {
    format!(
        "view-{}.json",
        blake3::hash(&serde_json::to_vec(&(daemon, selection)).expect("serializable selection"))
            .to_hex()
    )
}
pub async fn load(files: Arc<Files>, name: String) -> Result<Option<Checkpoint>, String> {
    tokio::task::spawn_blocking(move || {
        let file = match std::fs::File::open(files.root.join(name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let mut bytes = Vec::new();
        file.take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("Saved checkpoint exceeds the limit".into());
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}
pub async fn save(
    files: Arc<Files>,
    name: String,
    mut checkpoint: Checkpoint,
) -> Result<(), String> {
    checkpoint.resume.publication = None;
    for content in checkpoint.members.values_mut() {
        if let Content::Document(document) = content {
            document.streams.clear();
        }
    }
    tokio::task::spawn_blocking(move || {
        files.write(
            &name,
            &serde_json::to_vec(&checkpoint).map_err(|error| error.to_string())?,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}
#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        Node, Query,
        observation::{Document, Encoding, Member, Resume, Scope, ScopeId},
        sync::{Stream, Version},
    };
    use std::collections::BTreeMap;
    #[tokio::test]
    async fn persisted_checkpoint_is_canonical_and_fenced_by_owner_incarnation() {
        let files = Arc::new(
            Files::new(std::env::temp_dir().join(format!(
                "misa-scoped-checkpoint-{}",
                iroh::SecretKey::generate().public()
            )))
            .unwrap(),
        );
        let selection = Selection {
            scope: Scope {
                id: ScopeId::Session { id: "same".into() },
                incarnation: "one".into(),
            },
            members: BTreeMap::from([(
                "document".into(),
                Member {
                    query: Query::new("conversation"),
                    contract: "conversation@1".into(),
                    encoding: Encoding::Document,
                    optional: false,
                },
            )]),
        };
        let mut changed = selection.clone();
        changed.scope.incarnation = "two".into();
        assert_ne!(name("daemon", &selection), name("daemon", &changed));
        assert_ne!(name("daemon", &selection), name("other", &selection));
        let version = Version {
            epoch: "epoch".into(),
            rev: 3,
        };
        let checkpoint = Checkpoint {
            resume: Resume {
                selection: selection.clone(),
                publication: Some(7),
                documents: BTreeMap::from([("document".into(), version.clone())]),
            },
            members: BTreeMap::from([(
                "document".into(),
                Content::Document(Document {
                    version,
                    tree: Node::section("root").id("root"),
                    streams: vec![Stream {
                        id: "live".into(),
                        role: "message".into(),
                        text: "unfinished secret".into(),
                    }],
                }),
            )]),
        };
        let file = name("daemon", &selection);
        save(files.clone(), file.clone(), checkpoint).await.unwrap();
        let loaded = load(files.clone(), file.clone()).await.unwrap().unwrap();
        assert!(loaded.resume.publication.is_none());
        assert!(
            !std::fs::read_to_string(files.root.join(file))
                .unwrap()
                .contains("unfinished secret")
        );
        std::fs::remove_dir_all(&files.root).unwrap();
    }
}
