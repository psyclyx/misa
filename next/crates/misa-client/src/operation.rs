//! Accepted work is tracked by its exact owner reference, never by session idle.
use crate::{
    driver::{Client, Observation},
    interface::Interface,
};
use misa_proto::{Fault, Node, invocation::OperationRef, observation::Selection};
use misa_protocol::observation::{MemberState, Status};
use misa_value::Value;
use std::collections::BTreeMap;

#[derive(Debug)]
pub enum Terminal {
    Finished {
        state: String,
        outputs: Vec<i64>,
        document: Option<Node>,
        value: Value,
    },
    Expired,
    Fault(Fault),
}
#[derive(Debug)]
pub struct Completion {
    pub operation: OperationRef,
    pub generation: Option<i64>,
    pub outcome: Terminal,
}
pub struct Watch {
    operation: OperationRef,
    observation: Option<Observation>,
    failure: Option<Fault>,
    generation: Option<i64>,
    document: bool,
}
impl Watch {
    pub fn failed(operation: OperationRef, fault: Fault) -> Self {
        Self {
            operation,
            observation: None,
            failure: Some(fault),
            generation: None,
            document: false,
        }
    }
    pub async fn open(
        client: &Client,
        interface: &Interface,
        operation: OperationRef,
        document: bool,
    ) -> Result<Self, Fault> {
        let target = if operation.scope != interface.scope {
            Some(Interface::load(client, operation.scope.clone()).await?)
        } else {
            None
        };
        let interface = target.as_ref().unwrap_or(interface);
        let mut members = BTreeMap::from([(
            "result".into(),
            interface.query("operation.result", vec![Value::str(&operation.id)])?,
        )]);
        if document {
            members.insert(
                "document".into(),
                interface.query("operation.presentation", vec![Value::str(&operation.id)])?,
            );
        }
        let observation = client
            .observe(
                Selection {
                    scope: operation.scope.clone(),
                    members,
                },
                None,
            )
            .await?;
        Ok(Self {
            operation,
            observation: Some(observation),
            failure: None,
            generation: None,
            document,
        })
    }
    pub fn document(&self) -> Option<Node> {
        self.observation
            .as_ref()?
            .inspect(|replica, _| match replica.current()?.get("document")? {
                MemberState::Document(document) => Some(document.snapshot().tree),
                _ => None,
            })
            .flatten()
    }
    pub fn reference(&self) -> &OperationRef {
        &self.operation
    }
    pub fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.observation
            .as_ref()
            .map(Observation::watch)
            .unwrap_or_else(|| tokio::sync::watch::channel(0).1)
    }
    pub fn poll(&mut self) -> Option<Completion> {
        if let Some(fault) = &self.failure {
            return Some(Completion {
                operation: self.operation.clone(),
                generation: self.generation,
                outcome: Terminal::Fault(fault.clone()),
            });
        }
        let outcome = self
            .observation
            .as_ref()?
            .inspect(|replica, _| {
                match replica.status() {
                    Status::Closed(fault) => return Some(Terminal::Fault(fault.clone())),
                    Status::Stale(fault)
                        if !matches!(
                            fault.code.as_str(),
                            "observation.disconnected" | "observation.restored"
                        ) =>
                    {
                        return Some(Terminal::Fault(fault.clone()));
                    }
                    _ => {}
                }
                let members = replica.current()?;
                let value = match members.get("result") {
                    Some(MemberState::Value(value)) => value,
                    Some(MemberState::Unavailable(fault)) => {
                        return Some(Terminal::Fault(fault.clone()));
                    }
                    _ => {
                        return Some(Terminal::Fault(Fault::query(
                            "Operation result unavailable",
                        )));
                    }
                };
                if *value == Value::Null {
                    return Some(Terminal::Expired);
                }
                let result = decode_result(value, &self.operation.id, self.generation);
                let (generation, state, terminal, outputs) = match result {
                    Ok(result) => result,
                    Err(fault) => return Some(Terminal::Fault(fault)),
                };
                self.generation = Some(generation);
                if !terminal {
                    return None;
                }
                let document = if self.document {
                    match members.get("document") {
                        Some(MemberState::Document(document)) => Some(document.snapshot().tree),
                        _ => {
                            return Some(Terminal::Fault(Fault::query(
                                "Completed operation lacks coherent output",
                            )));
                        }
                    }
                } else {
                    None
                };
                Some(Terminal::Finished {
                    state,
                    outputs,
                    document,
                    value: value.clone(),
                })
            })
            .unwrap_or_else(|| {
                Some(Terminal::Fault(Fault::new(
                    "closed",
                    "Operation observation closed",
                )))
            });
        outcome.map(|outcome| Completion {
            operation: self.operation.clone(),
            generation: self.generation,
            outcome,
        })
    }
    pub async fn wait(mut self) -> Completion {
        loop {
            if let Some(completion) = self.poll() {
                return completion;
            }
            if let Err(fault) = self
                .observation
                .as_mut()
                .expect("nonfailed watch has observation")
                .changed()
                .await
            {
                return Completion {
                    operation: self.operation.clone(),
                    generation: self.generation,
                    outcome: Terminal::Fault(fault),
                };
            }
        }
    }
}
fn decode_result(
    value: &Value,
    id: &str,
    previous: Option<i64>,
) -> Result<(i64, String, bool, Vec<i64>), Fault> {
    let invalid = || Fault::query("Invalid or replaced operation result");
    if value.get("id").and_then(Value::as_str) != Some(id) {
        return Err(invalid());
    }
    let generation = value
        .get("generation")
        .and_then(Value::as_i64)
        .filter(|v| *v >= 0)
        .ok_or_else(invalid)?;
    if previous.is_some_and(|previous| previous > generation) {
        return Err(invalid());
    }
    let state = value
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let terminal = value
        .get("terminal")
        .and_then(Value::as_bool)
        .ok_or_else(invalid)?;
    if terminal
        != matches!(
            state,
            "succeeded" | "failed" | "cancelled" | "expired" | "interrupted"
        )
        || !matches!(
            state,
            "starting"
                | "queued"
                | "running"
                | "awaiting_input"
                | "submitting"
                | "cancelling"
                | "succeeded"
                | "failed"
                | "cancelled"
                | "expired"
                | "interrupted"
        )
    {
        return Err(invalid());
    }
    let outputs = match value.get("outputs") {
        None => &[][..],
        Some(Value::List(values)) => values.as_ref(),
        _ => return Err(invalid()),
    };
    let outputs = outputs
        .iter()
        .map(|value| value.as_i64().filter(|v| *v >= 0).ok_or_else(invalid))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((generation, state.into(), terminal, outputs))
}
/// Bounded live interests. Draining transfers terminal results to the caller;
/// this container never retains an unbounded terminal history.
pub struct Tracker {
    limit: usize,
    active: Vec<(Watch, tokio::task::JoinHandle<()>)>,
    signal: tokio::sync::watch::Sender<u64>,
    changes: tokio::sync::watch::Receiver<u64>,
}
impl Tracker {
    pub fn new(limit: usize) -> Self {
        let (signal, changes) = tokio::sync::watch::channel(0);
        Self {
            limit,
            active: vec![],
            signal,
            changes,
        }
    }
    pub fn latest_document(&self) -> Option<Node> {
        self.active
            .iter()
            .rev()
            .find_map(|(watch, _)| watch.document())
    }
    pub fn len(&self) -> usize {
        self.active.len()
    }
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
    pub fn insert(&mut self, watch: Watch) -> Result<(), Fault> {
        if self.active.len() >= self.limit {
            return Err(Fault::new("busy", "Too many tracked operations"));
        }
        if self
            .active
            .iter()
            .any(|(old, _)| old.reference() == watch.reference())
        {
            return Ok(());
        }
        let mut changes = watch.changes();
        let signal = self.signal.clone();
        let task = tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                signal.send_modify(|value| *value = value.wrapping_add(1));
            }
            signal.send_modify(|value| *value = value.wrapping_add(1));
        });
        self.active.push((watch, task));
        Ok(())
    }
    pub async fn changed(&mut self) -> Result<(), Fault> {
        self.changes
            .changed()
            .await
            .map_err(|_| Fault::new("closed", "Operation tracker closed"))
    }
    pub fn drain(&mut self) -> Vec<Completion> {
        let mut done = vec![];
        let mut index = 0;
        while index < self.active.len() {
            if let Some(result) = self.active[index].0.poll() {
                self.active.remove(index).1.abort();
                done.push(result);
            } else {
                index += 1;
            }
        }
        done
    }
}
impl Drop for Tracker {
    fn drop(&mut self) {
        for (_, task) in &self.active {
            task.abort();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn value(id: &str, generation: i64, state: &str, terminal: bool) -> Value {
        Value::map([
            ("id", Value::str(id)),
            ("generation", Value::Int(generation)),
            ("state", Value::str(state)),
            ("terminal", Value::Bool(terminal)),
            ("outputs", Value::list([Value::Int(3)])),
        ])
    }
    #[test]
    fn exact_identity_monotonic_generation_and_terminal_consistency() {
        assert!(decode_result(&value("a", 2, "awaiting_input", false), "a", Some(1)).is_ok());
        assert!(decode_result(&value("b", 2, "succeeded", true), "a", None).is_err());
        assert!(decode_result(&value("a", 1, "succeeded", true), "a", Some(2)).is_err());
        assert!(decode_result(&value("a", 2, "running", true), "a", None).is_err());
        assert!(decode_result(&value("a", 2, "interrupted", true), "a", None).is_ok());
    }
}
#[cfg(test)] mod tracker_tests {
 use super::*;
 use misa_proto::observation::{Scope,ScopeId};
 fn reference(id:&str)->OperationRef{OperationRef{scope:Scope{id:ScopeId::Session{id:"owner".into()},incarnation:"run".into()},id:id.into()}}
 #[tokio::test] async fn directed_failures_are_bounded_drained_once_and_keep_exact_owner(){
  let mut tracker=Tracker::new(1);
  tracker.insert(Watch::failed(reference("a"),Fault::new("lost","Accepted work could not be observed"))).unwrap();
  assert!(tracker.insert(Watch::failed(reference("b"),Fault::query("other"))).is_err());
  let done=tracker.drain();assert_eq!(done.len(),1);assert_eq!(done[0].operation,reference("a"));assert!(matches!(done[0].outcome,Terminal::Fault(_)));
  assert!(tracker.drain().is_empty());assert!(tracker.is_empty());
 }
}
#[cfg(test)]
#[path="operation_transport_tests.rs"]
mod transport_tests;
