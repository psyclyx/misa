//! Registered semantic indicators. Selection, layout, icons and styles belong to clients.
use std::sync::Arc;
use misa_proto::{Node, Query};
use misa_proto::view::{Action, Kind};
use misa_reframe::{Inputs, Registry, Subscription};
use misa_value::Value;

pub const MODEL_QUERY: &str = "presentation.indicators";
pub const DOCUMENT_QUERY: &str = "status.presentation";

pub fn definition() -> misa_proto::query::Definition {
    misa_proto::query::Definition { id: DOCUMENT_QUERY.into(), arguments: vec![], contract: "status.presentation@1".into(), result: misa_proto::query::ResultContract::Document {} }
}

#[derive(Clone, Debug)]
pub struct Indicator {
    pub id: String,
    pub label: String,
    pub query: Query,
    pub action: Option<Action>,
}

impl Indicator {
    pub fn new(id: &str, label: &str, query: &str) -> Self {
        Self { id: id.into(), label: label.into(), query: Query::new(query), action: None }
    }
}

/// One catalog for built-ins and contributed indicators; duplicate names are errors.
#[derive(Clone, Default)]
pub struct Catalog { entries: Vec<Indicator> }
impl Catalog {
    pub fn register(&mut self, indicator: Indicator) -> Result<(), String> {
        if indicator.id.is_empty() || indicator.query.id.is_empty() || self.entries.iter().any(|entry| entry.id == indicator.id) {
            return Err(format!("Invalid or duplicate indicator `{}`", indicator.id));
        }
        self.entries.push(indicator);
        Ok(())
    }

    /// Composition is an ordinary derived subscription, available to any consumer.
    pub fn subscription(&self) -> Subscription {
        let entries = self.entries.clone();
        Subscription::Derived {
            inputs: Inputs::Fixed(entries.iter().map(|entry| entry.query.clone()).collect()),
            compute: Arc::new(move |inputs, _, _| Ok(Value::list(entries.iter().zip(inputs).map(|(entry, fact)| Value::map([
                ("id", Value::str(&entry.id)), ("label", Value::str(&entry.label)), ("fact", fact.clone()),
            ]))))),
        }
    }

    pub fn document(&self) -> Subscription {
        let catalog = self.clone();
        misa_reframe::try_derived_query(Inputs::Fixed(vec![Query::new(MODEL_QUERY)]), move |inputs, _, _| {
            catalog.project(&inputs[0]).map(|node| crate::wire::render(&node)).map_err(misa_reframe::Fault::query)
        })
    }
    pub fn section(self, registry: Arc<Registry>) -> crate::views::Section {
        crate::views::Section::query("presentation.status", registry, Query::new(MODEL_QUERY), move |values| self.project(values))
    }
    fn project(&self, values: &Value) -> Result<Node, String> {
                let mut model = Node::section("status.indicators").id("indicators");
                for (definition, value) in self.entries.iter().zip(values.as_list().unwrap_or(&[])) {
                    let fact = value.get("fact").unwrap_or(&Value::Null);
                    if fact.is_null() { continue; }
                    let value = match fact_node(fact) {
                        Ok(value) => value,
                        Err(reason) => Node::new("error", Kind::Status { text: reason }),
                    };
                    let mut item = Node::section(format!("indicator.{}", definition.id)).id(&definition.id).label(&definition.label).child(value);
                    if let Some(action) = &definition.action { item.actions.push(action.clone()); }
                    model.children.push(item);
                }
                Ok(model)
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str { value.get(key).and_then(Value::as_str).unwrap_or("") }
fn scalar(kind: &str, value: Value) -> Value { Value::map([("type", Value::str(kind)), ("value", value)]) }
fn unavailable(reason: &str) -> Value { scalar("unavailable", Value::str(reason)) }

/// Translate typed facts to semantic wire primitives. No text formatting happens here.
fn fact_node(fact: &Value) -> Result<Node, String> {
    let kind = text(fact, "type");
    if kind.is_empty() { return Err("An indicator query must return a typed fact or null".into()); }
    if kind == "ratio" {
        let used = fact.get("used").and_then(Value::as_f64).ok_or("A ratio needs a used amount")?;
        let limit = fact.get("limit").and_then(Value::as_f64).ok_or("A ratio needs a limit")?;
        if !used.is_finite() || !limit.is_finite() || limit <= 0.0 { return Err("Invalid ratio bounds".into()); }
        return Ok(Node::new("value.ratio", Kind::Meter { label: String::new(), value: used, max: limit }));
    }
    let value = fact.get("value").cloned().ok_or("A scalar fact needs a value")?;
    if value.as_list().is_some() || value.as_map().is_some() { return Err("A scalar indicator fact must contain a scalar value".into()); }
    Ok(Node::new(format!("value.{kind}"), Kind::Fact { value }))
}

fn derived(inputs: &[&str], compute: impl Fn(&[Value]) -> Value + Send + Sync + 'static) -> Subscription {
    misa_reframe::derived_query(inputs.iter().map(|id| Query::new(*id)), compute)
}

pub fn subscriptions(registry: Registry) -> Registry {
    registry
        .subscription("status.activity", derived(&["session.status"], |inputs| scalar("activity", Value::str(text(&inputs[0], "status")))))
        .subscription("status.model", derived(&["session.status"], |inputs| scalar("text", Value::str(text(&inputs[0], "model")))))
        .subscription("status.effort", derived(&["session.status"], |inputs| {
            let value = text(&inputs[0], "effort");
            if value.is_empty() { Value::Null } else { scalar("text", Value::str(value)) }
        }))
        .subscription("status.turns", derived(&["session.status"], |inputs| scalar("count", inputs[0].get("turn").cloned().unwrap_or(Value::Int(0)))))
        .subscription("status.spend", derived(&["session.spend"], |inputs| scalar("money", inputs[0].clone())))
        .subscription("status.session", derived(&["usage.session"], |inputs| scalar("tokens", Value::Int(crate::usage::total_tokens(&inputs[0])))))
        .subscription("status.context", derived(&["session.status", "usage.last-request"], |inputs| {
            let used = crate::usage::total_tokens(&inputs[1]);
            match crate::catalog::model(text(&inputs[0], "model")) {
                Some(model) => Value::map([("type", Value::str("ratio")), ("used", Value::Int(used)), ("limit", Value::Int(model.context_window as i64))]),
                None => scalar("tokens", Value::Int(used)),
            }
        }))
        .subscription("status.plan", derived(&["usage.selected-quota"], |inputs| {
            if inputs[0].is_null() { Value::Null } else { plan(&inputs[0]) }
        }))
}

/// Missing quota is absent; an explicitly unavailable quota remains unavailable.
fn plan(usage: &Value) -> Value {
    if usage.get("unavailable").and_then(Value::as_bool) == Some(true) { return unavailable("provider unavailable"); }
    let remaining = usage.get("windows").and_then(Value::as_list).unwrap_or(&[]).iter().filter_map(|window| {
        let limit = window.get("limit")?.as_f64()?;
        let amount = window.get("remaining").and_then(Value::as_f64)
            .or_else(|| window.get("used").and_then(Value::as_f64).map(|used| limit - used))?;
        (limit.is_finite() && amount.is_finite() && limit > 0.0 && amount >= 0.0 && amount <= limit).then_some(100.0 * amount / limit)
    }).reduce(f64::min);
    remaining.map(|value| scalar("percent", Value::Float(value))).unwrap_or_else(|| unavailable("missing limit"))
}

pub fn builtins() -> Catalog {
    let mut catalog = Catalog::default();
    for (id, label, query) in [
        ("activity", "Activity", "status.activity"), ("model", "Model", "status.model"),
        ("effort", "Effort", "status.effort"), ("session", "Tokens", "status.session"),
        ("context", "Context", "status.context"), ("plan", "Plan", "status.plan"),
        ("turns", "Turns", "status.turns"), ("spend", "Spend", "status.spend"),
    ] { catalog.register(Indicator::new(id, label, query)).expect("unique built-in indicator"); }
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_reframe::Scope;
    #[test]
    fn a_contributed_indicator_uses_a_named_query_and_keeps_its_action() {
        let registry = Registry::new().subscription("power.remaining", derived(&[], |_| scalar("watts", Value::Int(7))));
        let mut catalog = Catalog::default();
        let mut indicator = Indicator::new("battery", "Battery", "power.remaining");
        indicator.action = Some(Action { id: "power.details".into(), on: misa_proto::view::ActionOn::Click, label: None, args: Value::Null });
        catalog.register(indicator.clone()).unwrap();
        assert!(catalog.register(indicator).is_err());
        let registry = Arc::new(registry.subscription(MODEL_QUERY, catalog.subscription()));
        let section = catalog.section(registry);
        let tree = (section.build)(&Value::Null).unwrap();
        misa_proto::view::validate(&tree).unwrap();
        let item = &tree.children[0];
        assert_eq!(item.role, "indicator.battery");
        assert_eq!(item.actions[0].id, "power.details");
        assert_eq!(item.children[0].kind, Kind::Fact { value: Value::Int(7) });
        assert_eq!(item.children[0].role, "value.watts");
    }
    #[test]
    fn quota_preserves_absence_unavailability_zero_and_the_tightest_window() {
        let window = |remaining, limit| Value::map([("remaining", Value::Int(remaining)), ("limit", Value::Int(limit))]);
        let usage = |windows| Value::map([("windows", Value::list(windows))]);
        assert_eq!(plan(&usage(vec![window(75, 100), window(1, 10)])).get("value").and_then(Value::as_f64), Some(10.0));
        assert_eq!(plan(&usage(vec![window(0, 100)])).get("value").and_then(Value::as_f64), Some(0.0));
        for bad in [window(1, 0), window(-1, 100), window(101, 100)] { assert_eq!(text(&plan(&usage(vec![bad])), "type"), "unavailable"); }
        assert_eq!(text(&plan(&Value::map([("unavailable", Value::Bool(true))])), "type"), "unavailable");
        let mut scope = Scope::new();
        let registry = subscriptions(crate::agent::registry());
        let query = Query::new("status.plan");
        scope.evaluate(&crate::views::initial_state("s", "p", "m", 0), &registry, &query).unwrap();
        assert_eq!(scope.current(&query), Some(Value::Null));
    }
}
