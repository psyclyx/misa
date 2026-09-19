//! Durable facts: the conversation log and the attempt ledger.
//!
//! Two tables, and the rules that matter are about *append*, not about schema:
//!
//! - the log is append-only, with a monotonic sequence per row. Nothing updates or
//!   deletes an entry, so a resumed conversation is the branch that was actually
//!   written rather than whatever the last process remembered.
//! - an attempt is written *before* the call it records and settled after. That is
//!   what makes "a request was made and nothing came back" a thing a resumed session
//!   can report instead of mistaking for a turn about to start.
//! - an attempt's name is `<conversation>/<request id>`, or the request id alone when
//!   there is no conversation, because a request id is only unique within the branch
//!   that issued it. A name a *finished* attempt already used is recorded beside it as
//!   `<name>#2`: a resumed session numbers its requests from the start again, and
//!   merging two rows would lose the newer attempt's cost. A name a *live* attempt
//!   still holds is refused, because two live attempts under one name is a mistake and
//!   not a resume.
//!
//! Entry `data` is stored as text, so the transcript model can evolve without a
//! migration. That is the previous system's decision and it has aged well.

use std::path::Path;

use misa_value::Value;

/// One durable fact.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub seq: i64,
    pub conversation: String,
    pub kind: String,
    pub data: Value,
}

/// One recorded model call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attempt {
    pub name: String,
    pub conversation: Option<String>,
    pub parent: Option<String>,
    pub kind: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_micros: i64,
    pub started_ms: i64,
    pub finished_ms: i64,
}

/// A conversation, as a list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub id: String,
    /// The first user message, which is what a conversation is about.
    pub title: String,
    pub messages: i64,
    pub last_ms: i64,
}

impl Conversation {
    pub fn to_value(&self) -> Value {
        Value::map([
            ("id", Value::str(&self.id)),
            ("title", Value::str(&self.title)),
            ("messages", Value::Int(self.messages)),
            ("last_ms", Value::Int(self.last_ms)),
        ])
    }
}

impl Attempt {
    pub fn to_value(&self) -> Value {
        Value::map([
            ("name", Value::str(&self.name)),
            ("kind", Value::str(&self.kind)),
            ("provider", Value::str(&self.provider)),
            ("model", Value::str(&self.model)),
            ("status", Value::str(&self.status)),
            ("input_tokens", Value::Int(self.input_tokens)),
            ("output_tokens", Value::Int(self.output_tokens)),
            ("cost_micros", Value::Int(self.cost_micros)),
            ("started_ms", Value::Int(self.started_ms)),
            ("finished_ms", Value::Int(self.finished_ms)),
        ])
    }
}

/// What a store is asked for.
pub trait Store: Send + Sync {
    fn append(
        &self,
        conversation: &str,
        kind: &str,
        data: &Value,
        at_ms: i64,
    ) -> Result<i64, String>;
    fn load(&self, conversation: &str, after: i64, limit: usize) -> Result<Vec<Entry>, String>;
    fn conversations(&self) -> Result<Vec<Conversation>, String>;
    /// Write a row before the call. Returns the name the row actually got.
    fn attempt_start(&self, attempt: &Attempt) -> Result<String, String>;
    fn attempt_settle(
        &self,
        name: &str,
        status: &str,
        input_tokens: i64,
        output_tokens: i64,
        cost_micros: i64,
        at_ms: i64,
    ) -> Result<(), String>;
    fn attempts(&self, conversation: Option<&str>) -> Result<Vec<Attempt>, String>;
    /// Whether an attempt under this name is still running.
    fn attempt_live(&self, name: &str) -> Result<bool, String>;
}

/// The naming rule, shared by both stores so it cannot drift between them.
///
/// Two questions, kept apart on purpose. A name a *finished* attempt used is reused with
/// a `#2` suffix: a resumed session numbers its requests from the start again, and
/// merging the two rows would lose the newer attempt's cost. A name a *live* attempt
/// holds is refused, because two live attempts under one name is a mistake and not a
/// resume — and a caller that cannot tell the two apart will quietly overwrite one.
pub fn free_name(
    live: &mut dyn FnMut(&str) -> Result<bool, String>,
    exists: &mut dyn FnMut(&str) -> Result<bool, String>,
    name: &str,
) -> Result<String, String> {
    if live(name)? {
        return Err(format!(
            "`{name}` is held by an attempt that has not finished"
        ));
    }
    if !exists(name)? {
        return Ok(name.to_string());
    }
    for suffix in 2..=1_000 {
        let candidate = format!("{name}#{suffix}");
        if !exists(&candidate)? && !live(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(format!("`{name}` has been used too many times"))
}

/// Facts in memory. What a test uses, and what a session with no data directory gets.
#[derive(Default)]
pub struct MemoryStore {
    entries: std::sync::Mutex<Vec<Entry>>,
    attempts: std::sync::Mutex<Vec<Attempt>>,
    next_seq: std::sync::atomic::AtomicI64,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }
}

impl Store for MemoryStore {
    fn append(
        &self,
        conversation: &str,
        kind: &str,
        data: &Value,
        at_ms: i64,
    ) -> Result<i64, String> {
        let seq = self
            .next_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "the log is poisoned".to_string())?;
        entries.push(Entry {
            seq,
            conversation: conversation.to_string(),
            kind: kind.to_string(),
            data: data.clone(),
        });
        let _ = at_ms;
        Ok(seq)
    }

    fn load(&self, conversation: &str, after: i64, limit: usize) -> Result<Vec<Entry>, String> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| "the log is poisoned".to_string())?;
        Ok(entries
            .iter()
            .filter(|entry| entry.conversation == conversation && entry.seq > after)
            .take(limit)
            .cloned()
            .collect())
    }

    fn conversations(&self) -> Result<Vec<Conversation>, String> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| "the log is poisoned".to_string())?;
        Ok(summarise(&entries))
    }

    fn attempt_start(&self, attempt: &Attempt) -> Result<String, String> {
        let mut attempts = self
            .attempts
            .lock()
            .map_err(|_| "the ledger is poisoned".to_string())?;
        let live: Vec<String> = attempts
            .iter()
            .filter(|row| row.status == "started")
            .map(|row| row.name.clone())
            .collect();
        let all: Vec<String> = attempts.iter().map(|row| row.name.clone()).collect();
        let mut is_live = |name: &str| Ok(live.iter().any(|live| live == name));
        let mut is_known = |name: &str| Ok(all.iter().any(|known| known == name));
        let name = free_name(&mut is_live, &mut is_known, &attempt.name)?;
        let mut row = attempt.clone();
        row.name = name.clone();
        attempts.push(row);
        Ok(name)
    }

    fn attempt_settle(
        &self,
        name: &str,
        status: &str,
        input_tokens: i64,
        output_tokens: i64,
        cost_micros: i64,
        at_ms: i64,
    ) -> Result<(), String> {
        let mut attempts = self
            .attempts
            .lock()
            .map_err(|_| "the ledger is poisoned".to_string())?;
        for row in attempts.iter_mut() {
            if row.name == name {
                row.status = status.to_string();
                // An omitted field keeps what was recorded: a write that only adds
                // usage must not relabel or cheapen an attempt.
                if input_tokens > 0 {
                    row.input_tokens = input_tokens;
                }
                if output_tokens > 0 {
                    row.output_tokens = output_tokens;
                }
                if cost_micros > 0 {
                    row.cost_micros = cost_micros;
                }
                if at_ms > 0 {
                    row.finished_ms = at_ms;
                }
            }
        }
        Ok(())
    }

    fn attempts(&self, conversation: Option<&str>) -> Result<Vec<Attempt>, String> {
        let attempts = self
            .attempts
            .lock()
            .map_err(|_| "the ledger is poisoned".to_string())?;
        Ok(attempts
            .iter()
            .filter(|row| match conversation {
                Some(conversation) => row.conversation.as_deref() == Some(conversation),
                None => true,
            })
            .cloned()
            .collect())
    }

    fn attempt_live(&self, name: &str) -> Result<bool, String> {
        let attempts = self
            .attempts
            .lock()
            .map_err(|_| "the ledger is poisoned".to_string())?;
        Ok(attempts
            .iter()
            .any(|row| row.name == name && row.status == "started"))
    }
}

/// The same facts, on disk.
///
/// WAL and a busy timeout so two daemons pointed at one directory serialise instead
/// of losing a write, which is the previous system's arrangement and the reason its
/// log survived a second process.
pub struct SqliteStore {
    connection: std::sync::Mutex<rusqlite::Connection>,
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<SqliteStore, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let connection = rusqlite::Connection::open(path).map_err(|err| err.to_string())?;
        connection
            .execute_batch(
                "pragma journal_mode = wal;
                 pragma synchronous = normal;
                 pragma busy_timeout = 5000;
                 create table if not exists entries (
                     seq integer primary key autoincrement,
                     conversation text not null,
                     kind text not null,
                     data text not null,
                     at_ms integer not null default 0
                 );
                 create index if not exists entries_branch on entries (conversation, seq);
                 create table if not exists attempts (
                     name text primary key,
                     conversation text,
                     parent text,
                     kind text not null,
                     provider text not null,
                     model text not null,
                     status text not null,
                     input_tokens integer not null default 0,
                     output_tokens integer not null default 0,
                     cost_micros integer not null default 0,
                     started_ms integer not null default 0,
                     finished_ms integer not null default 0
                 );",
            )
            .map_err(|err| err.to_string())?;
        Ok(SqliteStore {
            connection: std::sync::Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "the database is poisoned".to_string())
    }
}

impl Store for SqliteStore {
    fn append(
        &self,
        conversation: &str,
        kind: &str,
        data: &Value,
        at_ms: i64,
    ) -> Result<i64, String> {
        let connection = self.lock()?;
        // The data is stored as text so the transcript model can evolve without a
        // migration; a ciborium round trip keeps it a single column and byte-exact.
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(data, &mut bytes).map_err(|err| err.to_string())?;
        connection
            .execute(
                "insert into entries (conversation, kind, data, at_ms) values (?1, ?2, ?3, ?4)",
                rusqlite::params![conversation, kind, hex(&bytes), at_ms],
            )
            .map_err(|err| err.to_string())?;
        Ok(connection.last_insert_rowid())
    }

    fn load(&self, conversation: &str, after: i64, limit: usize) -> Result<Vec<Entry>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "select seq, conversation, kind, data from entries
                 where conversation = ?1 and seq > ?2 order by seq asc limit ?3",
            )
            .map_err(|err| err.to_string())?;
        let rows = statement
            .query_map(
                rusqlite::params![conversation, after, limit as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(|err| err.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, conversation, kind, data) = row.map_err(|err| err.to_string())?;
            out.push(Entry {
                seq,
                conversation,
                kind,
                data: unhex_value(&data),
            });
        }
        Ok(out)
    }

    fn conversations(&self) -> Result<Vec<Conversation>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "select e.conversation, e.data, e.seq,
                        (select count(*) from entries c where c.conversation = e.conversation and c.kind = 'message')
                 from entries e
                 where e.seq in (select min(seq) from entries where kind = 'message' group by conversation)",
            )
            .map_err(|err| err.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|err| err.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            let (id, data, _seq, messages) = row.map_err(|err| err.to_string())?;
            out.push(Conversation {
                id,
                title: title_of(&unhex_value(&data)),
                messages,
                last_ms: 0,
            });
        }
        Ok(out)
    }

    fn attempt_start(&self, attempt: &Attempt) -> Result<String, String> {
        let connection = self.lock()?;
        let mut is_live = |name: &str| -> Result<bool, String> {
            connection
                .query_row(
                    "select 1 from attempts where name = ?1 and status = 'started'",
                    rusqlite::params![name],
                    |_| Ok(true),
                )
                .or_else(|err| match err {
                    rusqlite::Error::QueryReturnedNoRows => Ok(false),
                    other => Err(other.to_string()),
                })
        };
        let mut is_known = |name: &str| -> Result<bool, String> {
            connection
                .query_row(
                    "select 1 from attempts where name = ?1",
                    rusqlite::params![name],
                    |_| Ok(true),
                )
                .or_else(|err| match err {
                    rusqlite::Error::QueryReturnedNoRows => Ok(false),
                    other => Err(other.to_string()),
                })
        };
        let name = free_name(&mut is_live, &mut is_known, &attempt.name)?;
        connection
            .execute(
                "insert into attempts (name, conversation, parent, kind, provider, model, status,
                                       input_tokens, output_tokens, cost_micros, started_ms, finished_ms)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
                rusqlite::params![
                    name,
                    attempt.conversation,
                    attempt.parent,
                    attempt.kind,
                    attempt.provider,
                    attempt.model,
                    "started",
                    attempt.input_tokens,
                    attempt.output_tokens,
                    attempt.cost_micros,
                    attempt.started_ms,
                ],
            )
            .map_err(|err| err.to_string())?;
        Ok(name)
    }

    fn attempt_settle(
        &self,
        name: &str,
        status: &str,
        input_tokens: i64,
        output_tokens: i64,
        cost_micros: i64,
        at_ms: i64,
    ) -> Result<(), String> {
        let connection = self.lock()?;
        // `max` rather than assignment for the numbers, so a settle that omits a field
        // keeps what was recorded instead of writing a zero over it.
        connection
            .execute(
                "update attempts set status = ?2,
                     input_tokens = max(input_tokens, ?3),
                     output_tokens = max(output_tokens, ?4),
                     cost_micros = max(cost_micros, ?5),
                     finished_ms = case when ?6 > 0 then ?6 else finished_ms end
                 where name = ?1",
                rusqlite::params![
                    name,
                    status,
                    input_tokens,
                    output_tokens,
                    cost_micros,
                    at_ms
                ],
            )
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    fn attempts(&self, conversation: Option<&str>) -> Result<Vec<Attempt>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "select name, conversation, parent, kind, provider, model, status,
                        input_tokens, output_tokens, cost_micros, started_ms, finished_ms
                 from attempts where (?1 is null or conversation = ?1) order by started_ms asc",
            )
            .map_err(|err| err.to_string())?;
        let rows = statement
            .query_map(rusqlite::params![conversation], |row| {
                Ok(Attempt {
                    name: row.get(0)?,
                    conversation: row.get(1)?,
                    parent: row.get(2)?,
                    kind: row.get(3)?,
                    provider: row.get(4)?,
                    model: row.get(5)?,
                    status: row.get(6)?,
                    input_tokens: row.get(7)?,
                    output_tokens: row.get(8)?,
                    cost_micros: row.get(9)?,
                    started_ms: row.get(10)?,
                    finished_ms: row.get(11)?,
                })
            })
            .map_err(|err| err.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|err| err.to_string())?);
        }
        Ok(out)
    }

    fn attempt_live(&self, name: &str) -> Result<bool, String> {
        let connection = self.lock()?;
        connection
            .query_row(
                "select 1 from attempts where name = ?1 and status = 'started'",
                rusqlite::params![name],
                |_| Ok(true),
            )
            .or_else(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Ok(false),
                other => Err(other.to_string()),
            })
    }
}

/// The title a conversation is listed under: its first user message.
fn title_of(entry: &Value) -> String {
    // An entry may be a whole message or a bare string: policy decides what to journal,
    // and a conversation should still get named by whatever the first thing said was.
    let owned;
    let text = match entry {
        Value::Str(text) => &**text,
        other => {
            owned = other
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            &owned
        }
    };
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.is_empty() {
        "untitled".into()
    } else {
        line.chars().take(60).collect()
    }
}

fn summarise(entries: &[Entry]) -> Vec<Conversation> {
    let mut seen: Vec<(String, i64, String)> = Vec::new();
    for entry in entries.iter().filter(|entry| entry.kind == "message") {
        match seen.iter_mut().find(|(id, _, _)| id == &entry.conversation) {
            Some((_, count, _)) => *count += 1,
            None => seen.push((
                entry.conversation.clone(),
                1,
                // The first entry that says something is what the conversation is about.
                entry
                    .data
                    .get("role")
                    .and_then(Value::as_str)
                    .map(|role| role.to_string())
                    .unwrap_or_default(),
            )),
        }
    }
    seen.into_iter()
        .map(|(id, messages, _)| Conversation {
            title: entries
                .iter()
                .find(|entry| entry.conversation == id && entry.kind == "message")
                .map(|entry| title_of(&entry.data))
                .unwrap_or_else(|| id.clone()),
            id,
            messages,
            last_ms: 0,
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn unhex_value(text: &str) -> Value {
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let digits: Vec<char> = text.chars().collect();
    for pair in digits.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        let value = pair[0].to_digit(16).zip(pair[1].to_digit(16));
        if let Some((high, low)) = value {
            bytes.push(((high << 4) | low) as u8);
        }
    }
    ciborium::de::from_reader(&bytes[..]).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(name: &str) -> Attempt {
        Attempt {
            name: name.to_string(),
            conversation: Some("c1".into()),
            kind: "turn".into(),
            provider: "scripted".into(),
            model: "m".into(),
            status: "started".into(),
            started_ms: 1,
            ..Attempt::default()
        }
    }

    fn exercise(store: &dyn Store) {
        let seq = store
            .append("c1", "message", &Value::str("hello there"), 10)
            .unwrap();
        assert_eq!(seq, 1);
        store
            .append("c1", "message", &Value::str("a reply"), 11)
            .unwrap();
        store
            .append("c2", "message", &Value::str("another branch"), 12)
            .unwrap();

        let page = store.load("c1", 0, 10).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].data.as_str(), Some("hello there"));
        assert_eq!(store.load("c1", 1, 10).unwrap().len(), 1);

        let conversations = store.conversations().unwrap();
        assert_eq!(conversations.len(), 2);
        let first = conversations.iter().find(|c| c.id == "c1").unwrap();
        assert_eq!(
            first.title, "hello there",
            "a conversation is named by its first message"
        );
        assert_eq!(first.messages, 2);

        let name = store.attempt_start(&attempt("r1")).unwrap();
        assert_eq!(name, "r1");
        assert!(store.attempt_live("r1").unwrap());
        // A live name is refused.
        assert!(store.attempt_start(&attempt("r1")).is_err());
        store.attempt_settle("r1", "ok", 10, 20, 5, 99).unwrap();
        assert!(!store.attempt_live("r1").unwrap());
        // A settled name is recorded beside the old one.
        let recycled = store.attempt_start(&attempt("r1")).unwrap();
        assert_eq!(
            recycled, "r1#2",
            "a resumed session's attempt overwrote the old one"
        );
        store
            .attempt_settle("r1#2", "ok", 100, 200, 0, 100)
            .unwrap();

        let rows = store.attempts(Some("c1")).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].input_tokens, 10);
        assert_eq!(rows[1].input_tokens, 100);

        // An omitted field keeps what was recorded rather than cheapening the attempt.
        store.attempt_settle("r1", "ok", 0, 0, 0, 0).unwrap();
        assert_eq!(store.attempts(Some("c1")).unwrap()[0].input_tokens, 10);
    }

    #[test]
    fn the_memory_store_keeps_the_same_rules() {
        exercise(&MemoryStore::new());
    }

    #[test]
    fn operation_metadata_never_becomes_a_conversation_title_or_message_count() {
        let stores: Vec<Box<dyn Store>> = vec![
            Box::new(MemoryStore::new()),
            Box::new(SqliteStore::open(std::path::Path::new(":memory:")).unwrap()),
        ];
        for store in stores {
            store
                .append("only-metadata", "operations.checkpoint", &Value::Null, 0)
                .unwrap();
            store
                .append("chat", "operations.checkpoint", &Value::Null, 0)
                .unwrap();
            store
                .append("chat", "message", &Value::str("actual prompt"), 1)
                .unwrap();
            store
                .append("chat", "operations.checkpoint", &Value::Null, 2)
                .unwrap();
            let conversations = store.conversations().unwrap();
            assert_eq!(conversations.len(), 1);
            assert_eq!(conversations[0].title, "actual prompt");
            assert_eq!(conversations[0].messages, 1);
        }
    }

    #[test]
    fn the_sqlite_store_keeps_the_same_rules_and_survives_a_reopen() {
        let directory = std::env::temp_dir().join(format!("misa-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("conversations.sqlite3");
        {
            let store = SqliteStore::open(&path).unwrap();
            exercise(&store);
        }
        // A second process sees what the first wrote, which is the whole point.
        let reopened = SqliteStore::open(&path).unwrap();
        assert_eq!(reopened.load("c1", 0, 10).unwrap().len(), 2);
        assert_eq!(reopened.attempts(Some("c1")).unwrap().len(), 2);
        assert_eq!(reopened.conversations().unwrap().len(), 2);
        assert!(!reopened.attempt_live("r1#2").unwrap());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_name_used_a_thousand_times_is_refused_rather_than_looping() {
        let mut live = |_name: &str| Ok(false);
        let mut known = |_name: &str| Ok(true);
        assert!(free_name(&mut live, &mut known, "r1").is_err());
        // And a live name in the suffix run is skipped rather than taken.
        let mut live = |name: &str| Ok(name == "r1#2");
        let mut known = |name: &str| Ok(name == "r1" || name == "r1#2");
        assert_eq!(free_name(&mut live, &mut known, "r1").unwrap(), "r1#3");
    }
}
