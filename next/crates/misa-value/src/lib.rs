//! The immutable value a re-frame loop stores.
//!
//! Two properties are load-bearing and everything else here follows from them.
//!
//! **Cloning shares.** `Value::clone` of a string, list, or map shares its
//! allocation, so a subscription may be handed the whole database and retain it
//! for a comparison without copying anything.
//!
//! **Writing reuses.** A patch rebuilds only the path it touches. Every branch it
//! does not touch keeps the exact allocation it had, so a consumer can tell
//! whether something changed by comparing pointers. That is what makes memoized
//! subscriptions affordable: a subscription's declared inputs are compared by
//! pointer first and only fall back to a value comparison when identity differs,
//! and identity is preserved for every subtree the last transaction did not
//! write.
//!
//! Maps are ordered. Iteration order is therefore a function of the data, not of
//! a hash seed, which is what lets a query be named by a canonical string and a
//! log line be reproduced from it. The cost is a map clone per write, bounded by
//! the map's own size; it is recorded here rather than hidden.
//!
//! There is no `PartialEq` shortcut: equality is structural. Callers that want to
//! know "did this change" should prefer [`Value::same`], which answers by pointer
//! wherever it can.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

mod patch;

pub use patch::{Op, PatchError, Path, Seg, apply, apply_one};

/// A JSON-shaped value with structure shared between copies.
#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// A string, shared.
    Str(Arc<str>),
    /// Opaque bytes. JSON cannot carry these; the wire format can, and blobs
    /// small enough to inline arrive here.
    Bytes(Arc<[u8]>),
    List(Arc<[Value]>),
    Map(Arc<BTreeMap<String, Value>>),
}

impl Value {
    pub fn str(value: impl AsRef<str>) -> Self {
        Value::Str(Arc::from(value.as_ref()))
    }

    pub fn list(items: impl IntoIterator<Item = Value>) -> Self {
        Value::List(items.into_iter().collect::<Vec<_>>().into())
    }

    pub fn map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Self {
        let mut out = BTreeMap::new();
        for (key, value) in entries {
            out.insert(key.to_string(), value);
        }
        Value::Map(Arc::new(out))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// A one-word type name, for diagnostics.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Bytes(_) => "bytes",
            Value::List(_) => "list",
            Value::Map(_) => "map",
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(v) => Some(*v),
            Value::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Map(v) => Some(v),
            _ => None,
        }
    }

    /// The value at one map key, or `None` for a non-map or a missing key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_map().and_then(|map| map.get(key))
    }

    /// The value at one list index.
    pub fn index(&self, index: usize) -> Option<&Value> {
        self.as_list().and_then(|list| list.get(index))
    }

    /// The value at a path built from map keys and list indices.
    pub fn get_path(&self, path: &Path) -> Option<&Value> {
        let mut cursor = self;
        for segment in path.segments() {
            cursor = match segment {
                Seg::Key(key) => cursor.get(key)?,
                Seg::Index(index) => cursor.index(*index as usize)?,
            };
        }
        Some(cursor)
    }

    /// Whether two values are the same allocation, or structurally equal.
    ///
    /// Pointers are checked first and at every level, so a large subtree that a
    /// patch did not touch answers without being walked.
    pub fn same(&self, other: &Value) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::List(a), Value::List(b)) => {
                Arc::ptr_eq(a, b)
                    || (a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.same(y)))
            }
            (Value::Map(a), Value::Map(b)) => {
                Arc::ptr_eq(a, b)
                    || (a.len() == b.len()
                        && a.iter()
                            .zip(b.iter())
                            .all(|((ka, va), (kb, vb))| ka == kb && va.same(vb)))
            }
            _ => false,
        }
    }

    /// Whether two values are literally the same allocation.
    ///
    /// This is the question a memoized consumer actually asks: not "are these
    /// equal" but "is this the same thing I already have". It is cheaper than
    /// [`Value::same`] because it never walks anything, and it is what a test
    /// should assert when it means to pin down structural sharing.
    pub fn shares(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => Arc::ptr_eq(a, b),
            (Value::Bytes(a), Value::Bytes(b)) => Arc::ptr_eq(a, b),
            (Value::List(a), Value::List(b)) => Arc::ptr_eq(a, b),
            (Value::Map(a), Value::Map(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// A deterministic textual name for this value.
    ///
    /// Used as a subscription scope key and in logs. It is a function of the
    /// data alone: map keys are sorted, floats print their shortest round-trip
    /// form, and strings are escaped, so two calls on the same data agree and two
    /// different values do not collide by delimiter smuggling.
    pub fn canonical_key(&self) -> String {
        let mut out = String::new();
        self.write_canonical(&mut out);
        out
    }

    fn write_canonical(&self, out: &mut String) {
        use fmt::Write as _;
        match self {
            Value::Null => out.push_str("n"),
            Value::Bool(v) => {
                let _ = write!(out, "b{v}");
            }
            Value::Int(v) => {
                let _ = write!(out, "i{v}");
            }
            Value::Float(v) => {
                let _ = write!(out, "f{v:?}");
            }
            Value::Str(v) => {
                let _ = write!(out, "s{v:?}");
            }
            Value::Bytes(v) => {
                let _ = write!(out, "y{}:", v.len());
                for byte in v.iter() {
                    let _ = write!(out, "{byte:02x}");
                }
            }
            Value::List(items) => {
                out.push('[');
                for item in items.iter() {
                    item.write_canonical(out);
                    out.push(',');
                }
                out.push(']');
            }
            Value::Map(map) => {
                out.push('{');
                for (key, value) in map.iter() {
                    let _ = write!(out, "{key:?}:");
                    value.write_canonical(out);
                    out.push(',');
                }
                out.push('}');
            }
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A compact form for diagnostics: never the canonical key form, which is
        // deliberately verbose and only meaningful to a machine.
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Int(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v:?}"),
            Value::Str(v) => write!(f, "{v:?}"),
            Value::Bytes(v) => write!(f, "<{} bytes>", v.len()),
            Value::List(items) => {
                f.write_str("[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Value::Map(map) => {
                f.write_str("{")?;
                for (index, (key, value)) in map.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{key}: {value}")?;
                }
                f.write_str("}")
            }
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Int(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::str(value)
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::Str(Arc::from(value))
    }
}

/// The empty value, so a message may omit a field that is `Null` and still
/// deserialize.
impl Default for Value {
    fn default() -> Self {
        Value::Null
    }
}

impl serde::Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(v) => serializer.serialize_bool(*v),
            Value::Int(v) => serializer.serialize_i64(*v),
            Value::Float(v) => serializer.serialize_f64(*v),
            Value::Str(v) => serializer.serialize_str(v),
            Value::Bytes(v) => serializer.serialize_bytes(v),
            Value::List(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items.iter() {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Value::Map(map) => {
                let mut out = serializer.serialize_map(Some(map.len()))?;
                for (key, value) in map.iter() {
                    out.serialize_entry(key, value)?;
                }
                out.end()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Value;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any value")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }

            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Int(v))
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
                i64::try_from(v)
                    .map(Value::Int)
                    .map_err(|_| serde::de::Error::custom("integer out of range"))
            }

            fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
                Ok(Value::Float(v))
            }

            fn visit_str<E>(self, v: &str) -> Result<Value, E> {
                Ok(Value::str(v))
            }

            fn visit_string<E>(self, v: String) -> Result<Value, E> {
                Ok(Value::Str(Arc::from(v)))
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Value, E> {
                Ok(Value::Bytes(Arc::from(v)))
            }

            fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Value, E> {
                Ok(Value::Bytes(Arc::from(v.into_boxed_slice())))
            }

            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_none<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
                <Value as serde::Deserialize>::deserialize(d)
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Value, A::Error> {
                let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(item) = seq.next_element::<Value>()? {
                    items.push(item);
                }
                Ok(Value::List(items.into()))
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Value, A::Error> {
                let mut out = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, Value>()? {
                    out.insert(key, value);
                }
                Ok(Value::Map(Arc::new(out)))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        Value::map([
            ("a", Value::Int(1)),
            ("nested", Value::map([("b", Value::str("two"))])),
            ("list", Value::list([Value::Int(1), Value::Int(2)])),
        ])
    }

    #[test]
    fn cloning_a_value_shares_its_allocation() {
        let value = sample();
        let clone = value.clone();
        assert!(value.shares(&clone));
    }

    #[test]
    fn an_untouched_branch_keeps_its_allocation() {
        let before = sample();
        let after = apply(
            &before,
            &[(Path::parse("a").unwrap(), Op::Set(Value::Int(2)))],
        )
        .unwrap();
        let (Value::Map(a), Value::Map(b)) = (&before, &after) else {
            panic!("expected maps")
        };
        assert!(a.get("nested").unwrap().shares(b.get("nested").unwrap()));
        assert!(a.get("list").unwrap().shares(b.get("list").unwrap()));
        assert!(matches!(b.get("a"), Some(Value::Int(2))));
        assert_eq!(a.get("a"), Some(&Value::Int(1)));
    }

    #[test]
    fn same_answers_by_pointer_before_walking() {
        let value = sample();
        let clone = value.clone();
        assert!(value.same(&clone));
        let changed = apply(
            &value,
            &[(Path::parse("a").unwrap(), Op::Set(Value::Int(9)))],
        )
        .unwrap();
        assert!(!value.same(&changed));
    }

    #[test]
    fn canonical_keys_distinguish_data_without_delimiters() {
        let a = Value::list([Value::str("a,b"), Value::str("")]);
        let b = Value::list([Value::str("a"), Value::str("b,")]);
        assert_ne!(a.canonical_key(), b.canonical_key());
        assert_eq!(a.canonical_key(), a.clone().canonical_key());
    }

    #[test]
    fn canonical_keys_are_stable_across_insertion_order() {
        let mut first = BTreeMap::new();
        first.insert("z".to_string(), Value::Int(1));
        first.insert("a".to_string(), Value::Int(2));
        let mut second = BTreeMap::new();
        second.insert("a".to_string(), Value::Int(2));
        second.insert("z".to_string(), Value::Int(1));
        assert_eq!(
            Value::Map(Arc::new(first)).canonical_key(),
            Value::Map(Arc::new(second)).canonical_key()
        );
    }

    #[test]
    fn every_value_round_trips_through_cbor() {
        let values = [
            Value::Null,
            Value::Bool(true),
            Value::Int(-7),
            Value::Float(1.5),
            Value::str("hello"),
            Value::Bytes(Arc::from(&b"raw"[..])),
            sample(),
            Value::list([Value::Null, Value::list([])]),
        ];
        for value in values {
            let mut bytes = Vec::new();
            ciborium::ser::into_writer(&value, &mut bytes).unwrap();
            let back: Value = ciborium::de::from_reader(&bytes[..]).unwrap();
            assert!(value.same(&back), "{value} did not round trip");
        }
    }
}
