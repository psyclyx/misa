//! Explicit, ordered patches.
//!
//! A patch is a list of `(path, op)` pairs applied in order; a later op sees the
//! earlier one's result. Every op names what it does, so nothing is inferred from
//! the *shape* of the incoming data.
//!
//! This is a deliberate departure from the previous system, where a patch was a
//! nested value and its meaning depended on the shape of what it met: maps merged
//! recursively, non-empty arrays replaced, empty arrays did nothing, and four
//! separate control sentinels existed for the cases those rules could not
//! express. Every one of those rules was a place to be surprised. Here merging is
//! opt-in ([`Op::Merge`]), appending is opt-in ([`Op::Append`]), replacement is
//! the default, and there is nothing that can be nested inside itself.
//!
//! Three properties are preserved from that system because they were right:
//!
//! - Empty patches do nothing, and a patch that changes nothing returns the
//!   original allocation.
//! - Bounds are checked against the container's real length at the moment of
//!   application, not against a length observed earlier.
//! - The root cannot be replaced wholesale. A root patch must merge, so a policy
//!   cannot discard state it does not know about by returning a database it built
//!   itself.

use std::fmt;

use crate::Value;

/// One step of a path: a map key or a list index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(u32),
}

/// A location inside a value.
///
/// Written `key.key[index].key`. An empty path is the root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Path(Vec<Seg>);

impl Path {
    pub fn root() -> Self {
        Path(Vec::new())
    }

    /// Parse `a.b[0].c`. Every segment before a `.` or `[` is a map key.
    ///
    /// Index syntax is required for list positions, so a map whose key is
    /// `"0"` and a list's first element cannot be confused for one another.
    pub fn parse(text: &str) -> Result<Self, PatchError> {
        let mut segments = Vec::new();
        let mut rest = text;
        if rest.is_empty() {
            return Ok(Path(segments));
        }
        loop {
            let (key, tail) = match rest.find('[') {
                Some(at) => (&rest[..at], &rest[at..]),
                None => (rest, ""),
            };
            if key.is_empty() {
                if segments.is_empty() && tail.starts_with('[') {
                    // A leading index, as in `[0].text`.
                } else if tail.is_empty() && !segments.is_empty() {
                    // A trailing dot.
                    return Err(PatchError::new(text, PatchErrorKind::BadPath));
                } else {
                    return Err(PatchError::new(text, PatchErrorKind::BadPath));
                }
            } else {
                for part in key.split('.') {
                    // A key segment may not contain index syntax: `a]b` is not a path.
                    if part.is_empty() || part.contains(']') {
                        return Err(PatchError::new(text, PatchErrorKind::BadPath));
                    }
                    segments.push(Seg::Key(part.to_string()));
                }
            }
            if tail.is_empty() {
                return Ok(Path(segments));
            }
            let close = tail.find(']').ok_or_else(|| PatchError::new(text, PatchErrorKind::BadPath))?;
            let digits = &tail[1..close];
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(PatchError::new(text, PatchErrorKind::BadPath));
            }
            let index: u32 = digits.parse().map_err(|_| PatchError::new(text, PatchErrorKind::BadPath))?;
            segments.push(Seg::Index(index));
            rest = &tail[close + 1..];
            if rest.is_empty() {
                return Ok(Path(segments));
            }
            if let Some(stripped) = rest.strip_prefix('.') {
                if stripped.is_empty() {
                    return Err(PatchError::new(text, PatchErrorKind::BadPath));
                }
                rest = stripped;
            } else if !rest.starts_with('[') {
                return Err(PatchError::new(text, PatchErrorKind::BadPath));
            }
        }
    }

    pub fn segments(&self) -> &[Seg] {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push_key(&mut self, key: impl Into<String>) {
        self.0.push(Seg::Key(key.into()));
    }

    pub fn push_index(&mut self, index: u32) {
        self.0.push(Seg::Index(index));
    }

    /// This path with `other`'s segments appended.
    pub fn join(&self, other: &Path) -> Path {
        let mut segments = self.0.clone();
        segments.extend_from_slice(&other.0);
        Path(segments)
    }

    /// The last segment, or `None` at the root.
    pub fn last(&self) -> Option<&Seg> {
        self.0.last()
    }

    /// This path without its last segment.
    pub fn parent(&self) -> Option<Path> {
        if self.0.is_empty() {
            None
        } else {
            Some(Path(self.0[..self.0.len() - 1].to_vec()))
        }
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str(".");
        }
        for (position, segment) in self.0.iter().enumerate() {
            match segment {
                Seg::Key(key) => {
                    if position > 0 {
                        f.write_str(".")?;
                    }
                    f.write_str(key)?;
                }
                Seg::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

/// What a patch does at its path.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Replace the value at the path. Missing map keys are created.
    Set(Value),
    /// Recursively merge a map into the map at the path. Scalars and lists
    /// replace rather than merge, which is why this is a separate op.
    Merge(Value),
    /// Remove the map key or list element at the path.
    Delete,
    /// Grow the list at the path by one element.
    Append(Value),
    /// Grow the list at the path by every element of a non-empty list.
    AppendAll(Vec<Value>),
}

impl Op {
    /// The operation as data: one tag and the value it is about.
    ///
    /// The same shape the plugin world spells in WIT, and the one a log entry carries when a patch
    /// *is* the fact being recorded — nothing is inferred from the shape of a payload, which is
    /// this type's whole rule.
    pub fn to_value(&self) -> Value {
        match self {
            Op::Set(value) => Value::map([("set", value.clone())]),
            Op::Merge(value) => Value::map([("merge", value.clone())]),
            Op::Delete => Value::str("delete"),
            Op::Append(value) => Value::map([("append", value.clone())]),
            Op::AppendAll(items) => Value::map([("append-all", Value::list(items.clone()))]),
        }
    }

    /// The operation a value describes, or `None` when it describes none.
    pub fn from_value(value: &Value) -> Option<Op> {
        match value {
            Value::Str(tag) if tag.as_ref() == "delete" => Some(Op::Delete),
            Value::Map(map) if map.len() == 1 => {
                let (tag, payload) = map.iter().next()?;
                match tag.as_str() {
                    "set" => Some(Op::Set(payload.clone())),
                    "merge" => Some(Op::Merge(payload.clone())),
                    "append" => Some(Op::Append(payload.clone())),
                    "append-all" => payload.as_list().map(|items| Op::AppendAll(items.to_vec())),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The root a path starts at, when it starts at a key.
    ///
    /// A root is what a composition declares, so "which state is this patch about" is a question
    /// about the first segment and never about the text of a path.
    pub fn root_of(path: &Path) -> Option<&str> {
        match path.segments().first()? {
            Seg::Key(key) => Some(key.as_str()),
            Seg::Index(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchErrorKind {
    BadPath,
    /// The path descended into a scalar.
    NotAContainer,
    /// A list index was outside the list.
    OutOfBounds,
    /// `Append` or `AppendAll` met something that is not a list.
    NotAList,
    /// `Merge` was given something that is not a map.
    NotAMap,
    /// The whole database cannot be replaced; a root patch must merge.
    RootReplacement,
    /// The root has no parent, so it cannot be deleted.
    DeleteOfRoot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PatchError {
    path: String,
    kind: PatchErrorKind,
}

impl PatchError {
    fn new(path: impl Into<String>, kind: PatchErrorKind) -> Self {
        PatchError { path: path.into(), kind }
    }

    pub fn kind(&self) -> PatchErrorKind {
        self.kind
    }

    /// The path as it was written, for a diagnostic.
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            PatchErrorKind::BadPath => "not a valid path",
            PatchErrorKind::NotAContainer => "descends into a scalar",
            PatchErrorKind::OutOfBounds => "index out of bounds",
            PatchErrorKind::NotAList => "is not a list",
            PatchErrorKind::NotAMap => "is not a map",
            PatchErrorKind::RootReplacement => "replaces the whole database",
            PatchErrorKind::DeleteOfRoot => "deletes the whole database",
        };
        write!(f, "patch at `{}` {}", self.path, reason)
    }
}

impl std::error::Error for PatchError {}

/// Apply an ordered patch.
pub fn apply(root: &Value, ops: &[(Path, Op)]) -> Result<Value, PatchError> {
    let mut current = root.clone();
    for (path, op) in ops {
        current = apply_one(&current, path, op)?;
    }
    Ok(current)
}

/// Apply one op.
pub fn apply_one(root: &Value, path: &Path, op: &Op) -> Result<Value, PatchError> {
    if path.is_root() {
        return match op {
            Op::Merge(incoming) => merge_into(root, incoming, &path.to_string()),
            Op::Delete => Err(PatchError::new(path.to_string(), PatchErrorKind::DeleteOfRoot)),
            _ => Err(PatchError::new(path.to_string(), PatchErrorKind::RootReplacement)),
        };
    }
    descend(root, path.segments(), op, &path.to_string())
}

/// Return the new value for `node`, which is the container addressed by `segs`
/// or contains the addressed leaf.
fn descend(node: &Value, segs: &[Seg], op: &Op, shown: &str) -> Result<Value, PatchError> {
    if segs.is_empty() {
        return apply_here(node, op, shown);
    }

    let (head, tail) = segs.split_first().expect("non-empty");
    let existing = match head {
        Seg::Key(key) => node.get(key),
        Seg::Index(index) => node.index(*index as usize),
    };
    let existing = match existing {
        Some(value) => value.clone(),
        None => match (node, head) {
            // A path may only be created where it is written: an absent branch is
            // a leaf, never a container to descend through.
            _ if !tail.is_empty() => return Err(PatchError::new(shown, PatchErrorKind::NotAContainer)),
            (Value::Map(_) | Value::Null, Seg::Key(_)) => Value::Null,
            (Value::Null, Seg::Index(0)) => Value::Null,
            (Value::List(_), Seg::Index(_)) => return Err(PatchError::new(shown, PatchErrorKind::OutOfBounds)),
            _ => return Err(PatchError::new(shown, PatchErrorKind::NotAContainer)),
        },
    };

    // A delete belongs to the parent: it removes an entry instead of writing a
    // value, so it never descends into what it is removing.
    let removing = tail.is_empty() && matches!(op, Op::Delete);
    let written = if removing {
        None
    } else {
        let written = descend(&existing, tail, op, shown)?;
        if written.same(&existing) {
            // A write that produced the same value leaves this container — and
            // therefore every one of its siblings — alone.
            return Ok(node.clone());
        }
        Some(written)
    };

    match head {
        Seg::Key(key) => {
            let mut next = match node {
                Value::Map(map) => (**map).clone(),
                Value::Null => std::collections::BTreeMap::new(),
                _ => return Err(PatchError::new(shown, PatchErrorKind::NotAContainer)),
            };
            match written {
                Some(value) => {
                    next.insert(key.clone(), value);
                }
                None => {
                    if next.remove(key).is_none() {
                        return Ok(node.clone());
                    }
                }
            }
            Ok(Value::Map(std::sync::Arc::new(next)))
        }
        Seg::Index(index) => {
            let mut next = match node {
                Value::List(list) => list.to_vec(),
                Value::Null if *index == 0 => Vec::new(),
                _ => return Err(PatchError::new(shown, PatchErrorKind::OutOfBounds)),
            };
            let at = *index as usize;
            match written {
                Some(value) if at == next.len() => next.push(value),
                Some(value) if at < next.len() => next[at] = value,
                Some(_) => return Err(PatchError::new(shown, PatchErrorKind::OutOfBounds)),
                None if at < next.len() => {
                    next.remove(at);
                }
                None => return Err(PatchError::new(shown, PatchErrorKind::OutOfBounds)),
            }
            Ok(Value::List(next.into()))
        }
    }
}

/// Apply an op to the node it is written at, replacing it or growing its list.
fn apply_here(node: &Value, op: &Op, shown: &str) -> Result<Value, PatchError> {
    match op {
        Op::Set(value) => Ok(value.clone()),
        Op::Merge(incoming) => merge_into(node, incoming, shown),
        // Unreachable through `apply_one`, which refuses a root delete before
        // descending. Kept so the op set is total here.
        Op::Delete => Err(PatchError::new(shown, PatchErrorKind::DeleteOfRoot)),
        Op::Append(value) => match node {
            Value::Null => Ok(Value::list([value.clone()])),
            Value::List(list) => {
                let mut next = list.to_vec();
                next.push(value.clone());
                Ok(Value::List(next.into()))
            }
            _ => Err(PatchError::new(shown, PatchErrorKind::NotAList)),
        },
        Op::AppendAll(values) => {
            if values.is_empty() {
                return Ok(node.clone());
            }
            match node {
                Value::Null => Ok(Value::list(values.clone())),
                Value::List(list) => {
                    let mut next = list.to_vec();
                    next.extend(values.iter().cloned());
                    Ok(Value::List(next.into()))
                }
                _ => Err(PatchError::new(shown, PatchErrorKind::NotAList)),
            }
        }
    }
}
fn merge_into(node: &Value, incoming: &Value, shown: &str) -> Result<Value, PatchError> {
    let Value::Map(incoming) = incoming else {
        return Err(PatchError::new(shown, PatchErrorKind::NotAMap));
    };
    let mut next = match node {
        Value::Map(map) => (**map).clone(),
        Value::Null => std::collections::BTreeMap::new(),
        _ => return Err(PatchError::new(shown, PatchErrorKind::NotAMap)),
    };
    for (key, value) in incoming.iter() {
        match (next.get(key), value) {
            (Some(Value::Map(_)), Value::Map(_)) => {
                let merged = merge_into(next.get(key).expect("checked"), value, shown)?;
                next.insert(key.clone(), merged);
            }
            _ => {
                next.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(Value::Map(std::sync::Arc::new(next)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Value {
        Value::map([
            ("session", Value::map([("status", Value::str("idle")), ("turn", Value::Int(0))])),
            ("messages", Value::list([Value::str("one"), Value::str("two")])),
            ("scratch", Value::Null),
        ])
    }

    fn path(text: &str) -> Path {
        Path::parse(text).unwrap()
    }

    #[test]
    fn paths_round_trip() {
        assert_eq!(path("").segments(), &[]);
        assert_eq!(path("a.b").segments(), &[Seg::Key("a".into()), Seg::Key("b".into())]);
        assert_eq!(path("a[2].b").segments(), &[Seg::Key("a".into()), Seg::Index(2), Seg::Key("b".into())]);
        assert_eq!(path("[0]").segments(), &[Seg::Index(0)]);
        assert!(Path::parse("a..b").is_err());
        assert!(Path::parse("a.").is_err());
        assert!(Path::parse("a[x]").is_err());
        assert!(Path::parse("a[1").is_err());
        assert!(Path::parse("a]b").is_err());
    }

    #[test]
    fn an_empty_patch_changes_nothing_and_shares_everything() {
        let before = db();
        let after = apply(&before, &[]).unwrap();
        assert!(before.same(&after));
    }

    #[test]
    fn a_write_that_changes_nothing_returns_the_original() {
        let before = db();
        let after = apply(&before, &[(path("session.status"), Op::Set(Value::str("idle")))]).unwrap();
        assert!(before.shares(&after));
    }

    #[test]
    fn set_creates_a_missing_key() {
        let after = apply(&db(), &[(path("scratch.note"), Op::Set(Value::str("hi")))]).unwrap();
        assert_eq!(after.get_path(&path("scratch.note")).and_then(Value::as_str), Some("hi"));
    }

    #[test]
    fn delete_removes_a_key_and_a_list_element() {
        let after = apply(
            &db(),
            &[(path("session.turn"), Op::Delete), (path("messages[0]"), Op::Delete)],
        )
        .unwrap();
        assert!(after.get_path(&path("session.turn")).is_none());
        assert_eq!(
            after.get_path(&path("messages")).and_then(Value::as_list).map(<[Value]>::len),
            Some(1)
        );
        assert_eq!(after.get_path(&path("messages[0]")).and_then(Value::as_str), Some("two"));
    }

    #[test]
    fn append_grows_a_list_and_refuses_a_scalar() {
        let after = apply(&db(), &[(path("messages"), Op::Append(Value::str("three")))]).unwrap();
        assert_eq!(after.get_path(&path("messages[2]")).and_then(Value::as_str), Some("three"));
        assert!(matches!(
            apply(&db(), &[(path("session.status"), Op::Append(Value::str("x")))]),
            Err(PatchError { kind: PatchErrorKind::NotAList, .. })
        ));
    }

    #[test]
    fn append_all_keeps_order_and_refuses_an_empty_list() {
        let after = apply(
            &db(),
            &[(path("messages"), Op::AppendAll(vec![Value::str("three"), Value::str("four")]))],
        )
        .unwrap();
        let list = after.get_path(&path("messages")).and_then(Value::as_list).unwrap();
        assert_eq!(list.len(), 4);
        assert_eq!(list[3].as_str(), Some("four"));

        let before = db();
        let unchanged = apply(&before, &[(path("messages"), Op::AppendAll(vec![]))]).unwrap();
        assert!(before.same(&unchanged));
    }

    #[test]
    fn index_bounds_are_checked_against_the_real_list() {
        assert!(matches!(
            apply(&db(), &[(path("messages[9]"), Op::Set(Value::str("x")))]),
            Err(PatchError { kind: PatchErrorKind::OutOfBounds, .. })
        ));
    }

    #[test]
    fn a_path_may_not_descend_into_a_scalar() {
        assert!(matches!(
            apply(&db(), &[(path("session.status.deeper"), Op::Set(Value::str("x")))]),
            Err(PatchError { kind: PatchErrorKind::NotAContainer, .. })
        ));
    }

    #[test]
    fn merge_is_opt_in_and_recursive_only_over_maps() {
        let after = apply(
            &db(),
            &[(path("session"), Op::Merge(Value::map([("status", Value::str("busy"))])))],
        )
        .unwrap();
        assert_eq!(after.get_path(&path("session.status")).and_then(Value::as_str), Some("busy"));
        // The key the patch did not mention is still there: that is what merging
        // buys, and why it is never implicit.
        assert_eq!(after.get_path(&path("session.turn")).and_then(Value::as_i64), Some(0));
    }

    #[test]
    fn the_root_may_only_be_merged() {
        assert!(matches!(
            apply_one(&db(), &Path::root(), &Op::Set(Value::map([]))),
            Err(PatchError { kind: PatchErrorKind::RootReplacement, .. })
        ));
        assert!(apply_one(&db(), &Path::root(), &Op::Merge(Value::map([("new", Value::Int(1))]))).is_ok());
    }

    #[test]
    fn ops_apply_in_order_and_are_never_reinterpreted() {
        // Appending to a key that does not exist yet creates a one-element list.
        let after = apply(
            &db(),
            &[
                (path("scratch.log"), Op::Append(Value::str("first"))),
                (path("scratch.log"), Op::Append(Value::str("second"))),
            ],
        )
        .unwrap();
        let log = after.get_path(&path("scratch.log")).and_then(Value::as_list).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[1].as_str(), Some("second"));

        // Appending to a scalar is refused rather than silently replacing it,
        // which is what "an op means exactly what it says" costs.
        assert!(matches!(
            apply(&db(), &[(path("session.status"), Op::Append(Value::str("x")))]),
            Err(PatchError { kind: PatchErrorKind::NotAList, .. })
        ));
    }

    #[test]
    fn untouched_siblings_keep_their_allocations() {
        let before = db();
        let after = apply(&before, &[(path("session.status"), Op::Set(Value::str("busy")))]).unwrap();
        assert!(before.get("messages").unwrap().shares(after.get("messages").unwrap()));
    }

    #[test]
    fn a_patch_operation_goes_to_data_and_comes_back() {
        // The shape a log entry carries when a patch is the fact being recorded, and the shape the
        // plugin world spells in WIT: one tag, one payload, nothing inferred.
        let ops = [
            Op::Set(Value::map([("a", Value::Int(1))])),
            Op::Merge(Value::str("x")),
            Op::Delete,
            Op::Append(Value::Bool(true)),
            Op::AppendAll(vec![Value::Int(1), Value::Int(2)]),
        ];
        for op in ops {
            let data = op.to_value();
            assert_eq!(Op::from_value(&data), Some(op.clone()), "{data:?}");
        }
        // And a value that is not an operation is not one.
        assert_eq!(Op::from_value(&Value::Null), None);
        assert_eq!(Op::from_value(&Value::str("set")), None);
        assert_eq!(Op::from_value(&Value::map([("nonsense", Value::Int(1))])), None);
        assert_eq!(Op::from_value(&Value::map([("append-all", Value::Int(1))])), None);
    }

    #[test]
    fn the_root_a_patch_is_about_is_its_first_segment() {
        // Which state a patch is about is a question about a parsed path, never about its text.
        assert_eq!(Op::root_of(&Path::parse("guest.turns[0].seen").unwrap()), Some("guest"));
        assert_eq!(Op::root_of(&Path::parse("guest").unwrap()), Some("guest"));
        assert_eq!(Op::root_of(&Path::parse("").unwrap()), None);
        assert_eq!(Op::root_of(&Path::parse("[0]").unwrap()), None);
    }
}
