//! Store-backed archive facts are refreshed off the owner lock. Queries only
//! filter the installed immutable snapshot; no transcript or IO runs in a read.
use misa_kernel::{Attempt, Conversation, Entry, Store as KernelStore};
use misa_proto::{
    Fault, Query,
    query::{Definition, ResultContract},
    schema::Schema,
};
use misa_value::Value;
use std::sync::Arc;
use tokio::sync::watch;
pub struct Store {
    inner: Arc<dyn KernelStore>,
    changed: watch::Sender<u64>,
}
impl Store {
    pub fn new(inner: Arc<dyn KernelStore>) -> Arc<Self> {
        let (changed, _) = watch::channel(0);
        Arc::new(Self { inner, changed })
    }
    pub fn watch(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
}
impl KernelStore for Store {
    fn append(&self, c: &str, k: &str, d: &Value, at: i64) -> Result<i64, String> {
        let result = self.inner.append(c, k, d, at)?;
        if k == "message" {
            self.changed.send_modify(|n| *n = n.wrapping_add(1));
        }
        Ok(result)
    }
    fn load(&self, c: &str, after: i64, limit: usize) -> Result<Vec<Entry>, String> {
        self.inner.load(c, after, limit)
    }
    fn conversations(&self) -> Result<Vec<Conversation>, String> {
        self.inner.conversations()
    }
    fn attempt_start(&self, a: &Attempt) -> Result<String, String> {
        self.inner.attempt_start(a)
    }
    fn attempt_settle(
        &self,
        n: &str,
        s: &str,
        i: i64,
        o: i64,
        c: i64,
        at: i64,
    ) -> Result<(), String> {
        self.inner.attempt_settle(n, s, i, o, c, at)
    }
    fn attempts(&self, c: Option<&str>) -> Result<Vec<Attempt>, String> {
        self.inner.attempts(c)
    }
    fn attempt_live(&self, n: &str) -> Result<bool, String> {
        self.inner.attempt_live(n)
    }
}
pub fn definitions() -> Vec<Definition> {
    let record = |fields: Vec<(&str, Schema)>| Schema::Record {
        fields: fields
            .into_iter()
            .map(|(id, schema)| {
                (
                    id.into(),
                    misa_proto::schema::Field {
                        schema,
                        optional: false,
                    },
                )
            })
            .collect(),
        allow_unknown: false,
    };
    let page = |item| {
        record(vec![
            (
                "items",
                Schema::List {
                    items: Box::new(item),
                },
            ),
            ("truncated", Schema::Bool),
        ])
    };
    let def = |id: &str, arguments, schema| Definition {
        id: id.into(),
        arguments,
        contract: format!("{id}@1"),
        result: ResultContract::Data { schema },
    };
    vec![
        def(
            "daemon.conversations",
            vec![Schema::String, Schema::Int],
            page(record(vec![
                ("id", Schema::String),
                ("title", Schema::String),
                ("messages", Schema::Int),
                ("last_ms", Schema::Int),
            ])),
        ),
        def(
            misa_proto::preparation::SOURCES,
            vec![],
            Schema::List {
                items: Box::new(Schema::Value),
            },
        ),
        def(
            misa_proto::preparation::SEARCH,
            vec![Schema::String, Schema::String, Schema::Int],
            page(record(vec![
                ("value", Schema::String),
                ("label", Schema::String),
                (
                    "detail",
                    Schema::Nullable {
                        inner: Box::new(Schema::String),
                    },
                ),
            ])),
        ),
    ]
}
pub fn sources() -> Value {
    let definition = definitions()
        .into_iter()
        .find(|d| d.id == misa_proto::preparation::SEARCH)
        .unwrap();
    crate::lifecycle::encode(&vec![misa_proto::preparation::Source {
        id: "conversations".into(),
        label: "Stored conversations".into(),
        kind: misa_proto::wire::SourceKind::OnDemand,
        member: definition
            .member(vec![
                Value::str("conversations"),
                Value::str(""),
                Value::Int(32),
            ])
            .unwrap(),
    }])
}
pub fn read(snapshot: &Result<Value, Fault>, query: &Query) -> Result<Value, Fault> {
    if query.id == misa_proto::preparation::SOURCES {
        return Ok(sources());
    }
    let completion = query.id == misa_proto::preparation::SEARCH;
    if completion && query.args.first().and_then(Value::as_str) != Some("conversations") {
        return Err(Fault::query("Unknown daemon completion source"));
    }
    let offset = usize::from(completion);
    let prefix = query.args[offset].as_str().unwrap().to_lowercase();
    let limit = query.args[offset + 1].as_i64().unwrap();
    if !(1..=200).contains(&limit) {
        return Err(Fault::query("Archive limit must be between 1 and 200"));
    }
    let snapshot = snapshot.as_ref().map_err(Clone::clone)?;
    let mut rows = snapshot
        .as_list()
        .unwrap_or(&[])
        .iter()
        .filter(|row| {
            ["id", "title"].iter().any(|key| {
                row.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&prefix)
            })
        })
        .take(limit as usize + 1)
        .cloned()
        .collect::<Vec<_>>();
    let truncated = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    if completion {
        rows = rows
            .into_iter()
            .map(|row| {
                Value::map([
                    ("value", row.get("id").cloned().unwrap()),
                    ("label", row.get("title").cloned().unwrap()),
                    (
                        "detail",
                        Value::str(format!(
                            "{} messages",
                            row.get("messages").and_then(Value::as_i64).unwrap_or(0)
                        )),
                    ),
                ])
            })
            .collect();
    }
    Ok(Value::map([
        ("items", Value::list(rows)),
        ("truncated", Value::Bool(truncated)),
    ]))
}
#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::observation::{Content, Selection};
    use misa_protocol::{
        invocation::{CallContext, CommandOwner},
        owner::Owner,
    };
    #[tokio::test]
    async fn empty_daemon_lists_stored_history_and_refreshes_after_committed_messages() {
        let store = Store::new(Arc::new(misa_kernel::MemoryStore::default()));
        store
            .append("old", "message", &Value::str("Archived prompt"), 1)
            .unwrap();
        let directory = crate::directory::Directory::new("archive-test").unwrap();
        directory.install_archive(store.clone()).await.unwrap();
        assert!(directory.sessions().is_empty());
        let context = CallContext {
            principal: "test".into(),
            connection: 1,
        };
        let definition = definitions()
            .into_iter()
            .find(|definition| definition.id == "daemon.conversations")
            .unwrap();
        let selection = Selection {
            scope: directory.scope(),
            members: std::collections::BTreeMap::from([(
                "archive".into(),
                definition
                    .member(vec![Value::str(""), Value::Int(1)])
                    .unwrap(),
            )]),
        };
        let snapshot = directory.read(&context, &selection).unwrap();
        let Content::Value(value) = &snapshot.members["archive"] else {
            panic!()
        };
        assert_eq!(
            value.get("items").and_then(Value::as_list).unwrap()[0].get("id"),
            Some(&Value::str("old"))
        );
        store
            .append("new", "message", &Value::str("Another prompt"), 2)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let snapshot = directory.read(&context, &selection).unwrap();
                if let Content::Value(value) = &snapshot.members["archive"]
                    && value.get("truncated") == Some(&Value::Bool(true))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let bad = Query::new("daemon.conversations")
            .arg(Value::str(""))
            .arg(Value::Int(1000));
        assert!(read(&Ok(Value::list([])), &bad).is_err());
        directory.shutdown_complete().await;
    }
}
