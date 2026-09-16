//! The query scope: declared-input queries, memoized.
//!
//! A subscription says what it depends on. The scope evaluates those
//! dependencies first, compares them with the ones it used last time, and
//! recomputes only when one of them is a different value. Because state is
//! immutable and shares structure, that comparison is a pointer test for every
//! branch a patch did not touch — which is nearly all of them.
//!
//! The scope is bounded. It is a cache, not an owner: eviction may cause a
//! recomputation, so a computation must never rely on running exactly once, and
//! `previous` is a hint that may be absent.

use std::collections::{HashMap, VecDeque};

use misa_proto::Query;
use misa_value::Value;

use crate::{Fault, Inputs, Registry, Subscription};

/// The cache size a scope starts with.
pub const DEFAULT_CAPACITY: usize = 512;
/// How deep a dependency chain may go before it is treated as a cycle.
pub const DEFAULT_DEPTH: usize = 64;

struct Entry {
    value: Value,
    inputs: Vec<Value>,
}

/// A bounded memo of query results.
pub struct Scope {
    entries: HashMap<String, Entry>,
    /// Most recently inserted last, so eviction takes the coldest.
    order: VecDeque<String>,
    capacity: usize,
    depth_limit: usize,
}

impl Default for Scope {
    fn default() -> Self {
        Scope::new()
    }
}

impl Scope {
    pub fn new() -> Self {
        Scope::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Scope {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
            depth_limit: DEFAULT_DEPTH,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The value this scope last computed for a query, if it still holds one.
    pub fn current(&self, query: &Query) -> Option<Value> {
        self.entries.get(&query.key()).map(|entry| entry.value.clone())
    }

    /// Drop one query's memo. The next evaluation recomputes it.
    pub fn forget(&mut self, query: &Query) {
        let key = query.key();
        self.entries.remove(&key);
        self.order.retain(|entry| entry != &key);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    /// Evaluate a query, returning the new value when it changed or is new.
    pub fn evaluate(
        &mut self,
        db: &Value,
        registry: &Registry,
        query: &Query,
    ) -> Result<Option<Value>, Fault> {
        self.eval(db, registry, query, &mut Vec::new())
    }

    fn eval(
        &mut self,
        db: &Value,
        registry: &Registry,
        query: &Query,
        active: &mut Vec<String>,
    ) -> Result<Option<Value>, Fault> {
        if active.len() >= self.depth_limit || active.contains(&query.key()) {
            return Err(Fault::query(format!(
                "`{}` is more than {} dependencies deep, which is a cycle or a mistake",
                query.id, self.depth_limit
            )));
        }
        let subscription = registry
            .definition(&query.id)
            .ok_or_else(|| Fault::query(format!("no subscription named `{}`", query.id)))?
            .clone();

        active.push(query.key());
        let result = (|| {
        let inputs = match subscription.as_ref() {
            // A read is invalidated by the database's own identity. Because a
            // patch reuses every branch it did not write, that comparison is a
            // pointer test for everything the last transaction left alone.
            Subscription::Read { .. } => vec![db.clone()],
            Subscription::Derived { inputs: Inputs::Fixed(fixed), .. } => self.evaluate_dependencies(db, registry, fixed, active, query)?,
            Subscription::Derived { inputs: Inputs::Dynamic(build), .. } => {
                let fixed = build(query)?;
                self.evaluate_dependencies(db, registry, &fixed, active, query)?
            }
        };
        let key = query.key();
        if let Some(entry) = self.entries.get(&key) {
            let unchanged = entry.inputs.len() == inputs.len()
                && entry.inputs.iter().zip(inputs.iter()).all(|(before, now)| before.same(now));
            if unchanged {
                return Ok(None);
            }
            let previous = entry.value.clone();
            let value = match subscription.as_ref() {
                Subscription::Read { read } => read(db, query, Some(&previous))?,
                Subscription::Derived { compute, .. } => compute(&inputs, query, Some(&previous))?,
            };
            if previous.same(&value) {
                self.insert(key, Entry { value: previous, inputs });
                return Ok(None);
            }
            self.insert(key, Entry { value: value.clone(), inputs });
            return Ok(Some(value));
        }

        let value = match subscription.as_ref() {
            Subscription::Read { read } => read(db, query, None)?,
            Subscription::Derived { compute, .. } => compute(&inputs, query, None)?,
        };
        self.insert(key, Entry { value: value.clone(), inputs });
        Ok(Some(value))
        })();
        active.pop();
        result
    }

    /// Evaluate each declared dependency and collect its current value.
    ///
    /// A dependency's own value is fetched from the scope rather than returned by
    /// the recursive call, because an unchanged dependency returns nothing — it
    /// has already been compared, and comparing again would be the work the memo
    /// exists to avoid.
    #[allow(clippy::too_many_arguments)]
    fn evaluate_dependencies(
        &mut self,
        db: &Value,
        registry: &Registry,
        dependencies: &[Query],
        active: &mut Vec<String>,
        query: &Query,
    ) -> Result<Vec<Value>, Fault> {
        let mut inputs = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            if dependency.id.is_empty() {
                return Err(Fault::query(format!(
                    "`{}` declared a dependency with no name",
                    query.id
                )));
            }
            self.eval(db, registry, dependency, active)?;
            inputs.push(
                self.current(dependency)
                    .ok_or_else(|| Fault::query(format!("`{}` has no value", dependency.id)))?,
            );
        }
        Ok(inputs)
    }

    fn insert(&mut self, key: String, entry: Entry) {
        if self.entries.insert(key.clone(), entry).is_none() {
            self.order.push_back(key);
        }
        while self.entries.len() > self.capacity {
            let Some(coldest) = self.order.pop_front() else {
                break;
            };
            if self.entries.remove(&coldest).is_none() {
                continue;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Subscription, read_query};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn recomputation_preserves_equal_output_allocation_and_suppresses_delivery() {
        let registry = Registry::new().subscription("constant", read_query(|_, _| Value::list([Value::Int(7)])));
        let mut scope = Scope::new();
        let query = Query::new("constant");
        let first = scope.evaluate(&Value::Int(1), &registry, &query).unwrap().unwrap();
        assert!(scope.evaluate(&Value::Int(2), &registry, &query).unwrap().is_none());
        let current = scope.current(&query).unwrap();
        match (first, current) {
            (Value::List(a), Value::List(b)) => assert!(Arc::ptr_eq(&a, &b)),
            _ => panic!("expected lists"),
        }
    }

    #[test]
    fn query_faults_propagate_and_can_recover_without_caching_a_false_value() {
        let registry = Registry::new()
            .subscription("input", crate::try_read_query(|db, _, _| {
                if db.is_null() { Err(Fault::query("temporarily missing")) } else { Ok(db.clone()) }
            }))
            .subscription("output", crate::derived_query([Query::new("input")], |inputs| inputs[0].clone()));
        registry.validate().unwrap();
        let mut scope = Scope::new();
        let query = Query::new("output");
        assert!(scope.evaluate(&Value::Null, &registry, &query).unwrap_err().message.contains("temporarily missing"));
        assert!(scope.current(&query).is_none());
        assert_eq!(scope.evaluate(&Value::Int(4), &registry, &query).unwrap(), Some(Value::Int(4)));
    }

    #[test]
    fn installation_rejects_duplicate_missing_and_cyclic_definitions() {
        let duplicate = Registry::new().subscription("x", read_query(|_, _| Value::Null))
            .subscription("x", read_query(|_, _| Value::Int(1)));
        assert!(duplicate.validate().unwrap_err().message.contains("duplicate"));
        let missing = Registry::new().subscription("x", crate::derived_query([Query::new("missing")], |_| Value::Null));
        assert!(missing.validate().unwrap_err().message.contains("missing"));
        let cycle = Registry::new().subscription("x", crate::derived_query([Query::new("y")], |_| Value::Null))
            .subscription("y", crate::derived_query([Query::new("x")], |_| Value::Null));
        assert!(cycle.validate().unwrap_err().message.contains("cycle"));
    }

    #[test]
    fn dynamic_cycles_fail_at_evaluation_and_do_not_poison_the_scope() {
        let registry = Registry::new().subscription("x", crate::try_derived_query(
            Inputs::Dynamic(Arc::new(|query| Ok(if query.args.is_empty() { vec![query.clone()] } else { vec![] }))),
            |_, _, _| Ok(Value::Int(1))));
        registry.validate().unwrap();
        let mut scope = Scope::new();
        assert!(scope.evaluate(&Value::Null, &registry, &Query::new("x")).is_err());
        assert_eq!(scope.evaluate(&Value::Null, &registry, &Query::new("x").arg(Value::Int(1))).unwrap(), Some(Value::Int(1)));
    }

    fn db() -> Value {
        Value::map([("items", Value::list([Value::Int(1), Value::Int(2)]))])
    }

    fn registry(calls: Arc<AtomicUsize>) -> Registry {
        Registry::new()
            .subscription(
                "items",
                read_query(|db, _query| db.get("items").cloned().unwrap_or(Value::Null)),
            )
            .subscription(
                "items.count",
                Subscription::Derived {
                    inputs: Inputs::Fixed(vec![Query::new("items")]),
                    compute: Arc::new(move |inputs, _query, _previous| Ok({
                        calls.fetch_add(1, Ordering::SeqCst);
                        Value::Int(inputs[0].as_list().map(<[Value]>::len).unwrap_or(0) as i64)
                    })),
                },
            )
            .subscription(
                "broken",
                Subscription::Derived {
                    inputs: Inputs::Fixed(vec![]),
                    compute: Arc::new(|_inputs, _query, _previous| Ok(Value::Null)),
                },
            )
            .subscription(
                "self",
                Subscription::Derived {
                    inputs: Inputs::Fixed(vec![Query::new("self")]),
                    compute: Arc::new(|_inputs, _query, _previous| Ok(Value::Null)),
                },
            )
            .subscription(
                "dynamic",
                Subscription::Derived {
                    inputs: Inputs::Dynamic(Arc::new(|query| Ok(vec![Query::new("items").arg(query.args[0].clone())]))),
                    compute: Arc::new(|_inputs, _query, _previous| Ok(Value::Str(Arc::from("dynamic")))),
                },
            )
    }

    #[test]
    fn a_new_query_reports_its_value_and_a_second_read_does_not() {
        let calls = Arc::new(AtomicUsize::new(0));
        let registry = registry(calls.clone());
        let mut scope = Scope::new();
        let query = Query::new("items.count");
        assert_eq!(scope.evaluate(&db(), &registry, &query).unwrap().unwrap().as_i64(), Some(2));
        assert_eq!(scope.evaluate(&db(), &registry, &query).unwrap(), None);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "an unchanged query recomputed");
    }

    #[test]
    fn a_changed_input_does_recompute() {
        let calls = Arc::new(AtomicUsize::new(0));
        let registry = registry(calls.clone());
        let mut scope = Scope::new();
        let query = Query::new("items.count");
        scope.evaluate(&db(), &registry, &query).unwrap();
        let bigger = Value::map([(
            "items",
            Value::list([Value::Int(1), Value::Int(2), Value::Int(3)]),
        )]);
        let reported = scope.evaluate(&bigger, &registry, &query).unwrap();
        assert_eq!(reported.unwrap().as_i64(), Some(3));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_missing_subscription_names_itself() {
        let registry = registry(Arc::new(AtomicUsize::new(0)));
        let mut scope = Scope::new();
        let error = scope.evaluate(&db(), &registry, &Query::new("nope")).unwrap_err();
        assert!(error.message.contains("nope"), "{error}");
    }

    #[test]
    fn a_dependency_cycle_is_reported_rather_than_hanging() {
        let registry = registry(Arc::new(AtomicUsize::new(0)));
        let mut scope = Scope::new();
        let error = scope.evaluate(&db(), &registry, &Query::new("self")).unwrap_err();
        assert!(error.message.contains("cycle or a mistake"), "{error}");
    }

    #[test]
    fn a_dynamic_dependency_is_validated_per_query() {
        let registry = registry(Arc::new(AtomicUsize::new(0)));
        let mut scope = Scope::new();
        let query = Query::new("dynamic").arg(Value::Int(1));
        assert_eq!(
            scope.evaluate(&db(), &registry, &query).unwrap().unwrap().as_str(),
            Some("dynamic")
        );
    }

    #[test]
    fn the_scope_is_bounded_and_evicts_the_coldest() {
        let registry = registry(Arc::new(AtomicUsize::new(0)));
        let mut scope = Scope::with_capacity(2);
        for index in 0..5 {
            scope
                .evaluate(&db(), &registry, &Query::new("items").arg(Value::Int(index)))
                .unwrap();
        }
        assert_eq!(scope.len(), 2);
        assert!(scope.capacity() == 2);
    }

    #[test]
    fn forgetting_a_query_recomputes_it() {
        let calls = Arc::new(AtomicUsize::new(0));
        let registry = registry(calls.clone());
        let mut scope = Scope::new();
        let query = Query::new("items.count");
        scope.evaluate(&db(), &registry, &query).unwrap();
        scope.forget(&query);
        assert!(scope.current(&query).is_none());
        assert!(scope.evaluate(&db(), &registry, &query).unwrap().is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_read_query_is_available_through_the_same_scope() {
        let registry = Registry::new().subscription(
            "echo",
            read_query(|db, query| {
                let key = query.args.first().and_then(Value::as_str).unwrap_or("items");
                db.get(key).cloned().unwrap_or(Value::Null)
            }),
        );
        let mut scope = Scope::new();
        let value = scope
            .evaluate(&db(), &registry, &Query::new("echo").arg(Value::str("items")))
            .unwrap()
            .unwrap();
        assert_eq!(value.as_list().map(<[Value]>::len), Some(2));
    }
}
