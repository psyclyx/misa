//! One fold for acknowledged live facts and replayed history.
use misa_value::{Op, Path, Value};

pub fn patches(db: &Value, kind: &str, data: &Value, log_seq: i64) -> Vec<(Path, Op)> {
    let mut out = Vec::new();
    let mut put = |path: &str, op| out.push((Path::parse(path).expect("journal path"), op));
    match kind {
        "reset" => {
            let replacement = data.get("message").cloned().into_iter().collect::<Vec<_>>();
            // A reset replaces the transcript, and the attempt ledger is the transcript's
            // ledger: the token and context indicators are read from it. A compaction
            // replaces the transcript with a summary and the branch continues, so its
            // accounting stays; a clear empties the transcript, and the accounting goes
            // with it. Both fall out of the reset's own data rather than the command name.
            if replacement.is_empty() {
                put("attempts", Op::Set(Value::list([])));
            }
            put("messages", Op::Set(Value::list(replacement)));
        }
        "message" => {
            if let Some(map) = data.as_map() {
                let mut map = map.clone();
                if !map.contains_key("seq") {
                    map.insert("seq".into(), Value::Int(log_seq));
                }
                // A message settles once. A second record carrying a seq the
                // transcript already holds is a race's residue — a cancel that
                // arrived after the answer it meant to stop — and folding it would
                // give one message two rows, which is one identity two nodes in the
                // view. The first settle stands; the log keeps the late record, the
                // fold does not, in a live acknowledgment and in a replay alike.
                let seq = map.get("seq").and_then(Value::as_i64);
                let settled = seq.is_some_and(|seq| {
                    db.get("messages")
                        .and_then(Value::as_list)
                        .unwrap_or(&[])
                        .iter()
                        .any(|row| row.get("seq").and_then(Value::as_i64) == Some(seq))
                });
                if !settled {
                    let attempt = map.remove("attempt");
                    put("messages", Op::Append(Value::Map(std::sync::Arc::new(map))));
                    if let Some(attempt) = attempt {
                        put("attempts", Op::Append(attempt));
                    }
                }
            }
        }
        "tool_result" => {
            let call = data.get("call").and_then(Value::as_str).unwrap_or_default();
            if let Some(messages) = db.get("messages").and_then(Value::as_list) {
                'found: for (index, message) in messages.iter().enumerate().rev() {
                    for (position, entry) in message
                        .get("calls")
                        .and_then(Value::as_list)
                        .unwrap_or(&[])
                        .iter()
                        .enumerate()
                    {
                        if entry.get("id").and_then(Value::as_str) == Some(call) {
                            let ok = data.get("ok").and_then(Value::as_bool).unwrap_or(false);
                            put(
                                &format!("messages[{index}].calls[{position}].status"),
                                Op::Set(Value::str(if ok { "ok" } else { "error" })),
                            );
                            put(
                                &format!("messages[{index}].calls[{position}].result"),
                                Op::Set(data.get("text").cloned().unwrap_or(Value::Null)),
                            );
                            break 'found;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

pub fn fold(mut db: Value, entries: &[Value]) -> Value {
    for entry in entries {
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let data = entry.get("data").unwrap_or(&Value::Null);
        let seq = entry.get("seq").and_then(Value::as_i64).unwrap_or(0);
        for (path, op) in patches(&db, kind, data, seq) {
            db = misa_value::apply_one(&db, &path, &op).expect("journal patch applies");
        }
    }
    db
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(seq: i64, kind: &str, data: Value) -> Value {
        Value::map([
            ("seq", Value::Int(seq)),
            ("kind", Value::str(kind)),
            ("data", data),
        ])
    }
    fn attempt(cost: i64) -> Value {
        Value::map([("cost_micros", Value::Int(cost))])
    }
    fn assistant(attempt: Value) -> Value {
        Value::map([
            ("role", Value::str("assistant")),
            ("text", Value::str("")),
            ("calls", Value::list([])),
            ("attempt", attempt),
        ])
    }
    #[test]
    fn live_acknowledgments_and_restart_fold_match_through_tools_and_compaction() {
        let base = Value::map([("messages", Value::list([])), ("attempts", Value::list([]))]);
        let entries = vec![
            entry(
                1,
                "message",
                Value::map([
                    ("role", Value::str("user")),
                    ("text", Value::str("question")),
                ]),
            ),
            entry(
                2,
                "message",
                Value::map([
                    ("role", Value::str("assistant")),
                    ("text", Value::str("")),
                    (
                        "calls",
                        Value::list([Value::map([
                            ("id", Value::str("call")),
                            ("status", Value::str("pending")),
                        ])]),
                    ),
                    ("attempt", Value::map([("cost_micros", Value::Int(12))])),
                ]),
            ),
            entry(
                3,
                "tool_result",
                Value::map([
                    ("call", Value::str("call")),
                    ("ok", Value::Bool(true)),
                    ("text", Value::str("result")),
                ]),
            ),
            entry(
                4,
                "reset",
                Value::map([(
                    "message",
                    Value::map([
                        ("seq", Value::Int(4)),
                        ("role", Value::str("system")),
                        ("text", Value::str("summary")),
                    ]),
                )]),
            ),
            entry(
                5,
                "message",
                Value::map([
                    ("role", Value::str("assistant")),
                    ("state", Value::str("cancelled")),
                    ("text", Value::str("partial")),
                ]),
            ),
        ];
        let mut live = base.clone();
        for (at, entry) in entries.iter().enumerate() {
            let ops = patches(
                &live,
                entry.get("kind").unwrap().as_str().unwrap(),
                entry.get("data").unwrap(),
                entry.get("seq").unwrap().as_i64().unwrap(),
            );
            live = misa_value::apply(&live, &ops).unwrap();
            assert_eq!(live, fold(base.clone(), &entries[..=at]));
            if at == 2 {
                assert_eq!(
                    live.get("messages").unwrap().as_list().unwrap()[1]
                        .get("calls")
                        .unwrap()
                        .as_list()
                        .unwrap()[0]
                        .get("result")
                        .unwrap()
                        .as_str(),
                    Some("result")
                );
            }
        }
        assert_eq!(live.get("messages").unwrap().as_list().unwrap().len(), 2);
        // Compaction replaces the transcript with a summary and the branch's
        // accounting survives, because the branch does.
        assert_eq!(live.get("attempts").unwrap().as_list().unwrap().len(), 1);
    }

    #[test]
    fn one_message_settles_once_however_the_writes_race() {
        let base = Value::map([("messages", Value::list([])), ("attempts", Value::list([]))]);
        let answer = entry(
            6,
            "message",
            Value::map([
                ("seq", Value::Int(2)),
                ("role", Value::str("assistant")),
                ("text", Value::str("done")),
                ("state", Value::str("done")),
                (
                    "calls",
                    Value::list([Value::map([
                        ("id", Value::str("toolu_racing")),
                        ("status", Value::str("pending")),
                    ])]),
                ),
                ("attempt", Value::map([("cost_micros", Value::Int(12))])),
            ]),
        );
        let late_cancel = entry(
            7,
            "message",
            Value::map([
                ("seq", Value::Int(2)),
                ("role", Value::str("assistant")),
                ("text", Value::str("partial")),
                ("state", Value::str("cancelled")),
                ("calls", Value::list([])),
            ]),
        );
        for (entries, kept, attempts) in [
            (
                vec![answer.clone(), late_cancel.clone()],
                "done",
                1,
                // The answer arrived first and stands, accounting and all.
            ),
            (
                vec![late_cancel.clone(), answer.clone()],
                "cancelled",
                0,
                // The cancel landed first; the late answer adds nothing.
            ),
        ] {
            let mut live = base.clone();
            for (at, entry) in entries.iter().enumerate() {
                let ops = patches(
                    &live,
                    entry.get("kind").unwrap().as_str().unwrap(),
                    entry.get("data").unwrap(),
                    entry.get("seq").unwrap().as_i64().unwrap(),
                );
                live = misa_value::apply(&live, &ops).unwrap();
                assert_eq!(live, fold(base.clone(), &entries[..=at]));
            }
            let messages = live.get("messages").unwrap().as_list().unwrap();
            assert_eq!(messages.len(), 1, "one message settles once");
            assert_eq!(messages[0].get("state").unwrap().as_str(), Some(kept));
            assert_eq!(messages[0].get("seq").unwrap(), &Value::Int(2));
            assert_eq!(
                live.get("attempts").unwrap().as_list().unwrap().len(),
                attempts
            );
        }
    }

    #[test]
    fn a_clear_empties_the_ledger_its_transcript_was_measured_from() {
        // A clear is a reset that installs nothing: the token and context indicators
        // must show the conversation that is left, which is none of them.
        let base = Value::map([("messages", Value::list([])), ("attempts", Value::list([]))]);
        let entries = vec![
            entry(1, "message", assistant(attempt(12))),
            entry(2, "reset", Value::map([("reason", Value::str("cleared"))])),
            entry(3, "message", assistant(attempt(3))),
        ];
        let folded = fold(base, &entries);
        assert_eq!(folded.get("messages").unwrap().as_list().unwrap().len(), 1);
        assert_eq!(
            folded.get("attempts").unwrap().as_list().unwrap(),
            &vec![attempt(3)],
            "the cleared attempt is still counted"
        );
    }
}
