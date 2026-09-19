//! Scoped observations of a session's installed exports.
//!
//! A handle owns its read cursor and comparison values. The runtime owns state,
//! shared query computation and canonical history. No observation owns a second
//! document tree, and no token copies the existing transcript.
use std::{collections::BTreeMap, sync::Arc};

use misa_proto::{
    Fault,
    observation::*,
    query::{Definition, ResultContract},
    schema::{Field, Schema},
    sync::ViewSync,
};
use misa_value::Value;
use tokio::sync::watch;

use crate::{Runtime, State};
use misa_protocol::invocation::CallContext;
pub(crate) type RestrictedQuery =
    fn(&State, &misa_proto::Query, &CallContext) -> Result<Value, Fault>;

#[cfg(test)]
thread_local! { static DOCUMENT_PARSES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
#[path = "observation_query_tests.rs"]
mod query_tests;

pub const CONVERSATION: &str = "conversation.presentation";
pub const SUMMARY: &str = "session.summary";

pub(crate) fn builtins() -> Vec<Definition> {
    let text = || Field {
        schema: Schema::String,
        optional: false,
    };
    vec![
        Definition {
            id: CONVERSATION.into(),
            arguments: vec![],
            contract: "conversation.presentation@1".into(),
            result: ResultContract::Document {},
        },
        Definition {
            id: SUMMARY.into(),
            arguments: vec![],
            contract: "session.summary@1".into(),
            result: ResultContract::Data {
                schema: Schema::Record {
                    fields: ["id", "activity", "provider", "model"]
                        .into_iter()
                        .map(|key| (key.into(), text()))
                        .chain([
                            (
                                "working".into(),
                                Field {
                                    schema: Schema::Bool,
                                    optional: false,
                                },
                            ),
                            (
                                "attention".into(),
                                Field {
                                    schema: Schema::Int,
                                    optional: false,
                                },
                            ),
                            (
                                "usage".into(),
                                Field {
                                    schema: Schema::Record {
                                        fields: ["input_tokens", "output_tokens", "cost_micros"]
                                            .into_iter()
                                            .map(|key| {
                                                (
                                                    key.into(),
                                                    Field {
                                                        schema: Schema::Int,
                                                        optional: false,
                                                    },
                                                )
                                            })
                                            .collect(),
                                        allow_unknown: false,
                                    },
                                    optional: false,
                                },
                            ),
                            (
                                "operations".into(),
                                Field {
                                    schema: Schema::List {
                                        items: Box::new(Schema::Value),
                                    },
                                    optional: false,
                                },
                            ),
                            (
                                "requests".into(),
                                Field {
                                    schema: Schema::List {
                                        items: Box::new(Schema::Value),
                                    },
                                    optional: false,
                                },
                            ),
                        ])
                        .collect(),
                    allow_unknown: false,
                },
            },
        },
    ]
}

/// The receiver coalesces publications. Reading after wakeup captures all members
/// under the runtime lock; registration and the initial read use that same lock.
pub struct Observation {
    context: Option<CallContext>,
    runtime: Arc<Runtime>,
    handle: Handle,
    selection: Selection,
    changed: watch::Receiver<u64>,
    resume: Option<Resume>,
    values: BTreeMap<String, Cached>,
    examined: Option<u64>,
    fault: Option<Fault>,
    ended: bool,
}

impl misa_protocol::owner::Observation for Observation {
    fn changed(&mut self) -> &mut watch::Receiver<u64> {
        &mut self.changed
    }
    fn poll(&mut self) -> Option<Publication> {
        Observation::poll(self)
    }
}

impl Runtime {
    pub fn scope(&self) -> Scope {
        Scope {
            id: ScopeId::Session {
                id: self.id().into(),
            },
            incarnation: self.incarnation.clone(),
        }
    }

    pub fn query_exports(&self) -> Vec<Definition> {
        self.exports.values().cloned().collect()
    }

    /// Transport admission must authorize the caller before lending this owner.
    /// Every read also checks exact owner incarnation and installed contracts.
    pub fn observe(
        self: &Arc<Self>,
        handle: Handle,
        selection: Selection,
        resume: Option<Resume>,
    ) -> Result<(Observation, Publication), Fault> {
        self.observe_with_context(None, handle, selection, resume)
    }

    pub(crate) fn observe_with_context(
        self: &Arc<Self>,
        context: Option<CallContext>,
        handle: Handle,
        selection: Selection,
        resume: Option<Resume>,
    ) -> Result<(Observation, Publication), Fault> {
        self.validate_selection(&selection)?;
        if resume
            .as_ref()
            .is_some_and(|resume| resume.selection != selection)
        {
            return Err(Fault::query(
                "Resume selection does not match this observation",
            ));
        }
        let changed = {
            let state = self.state.lock().expect("session state is never poisoned");
            if resume
                .as_ref()
                .and_then(|resume| resume.publication)
                .is_some_and(|position| position > state.publications.position())
            {
                return Err(Fault::query("Resume publication is ahead of its owner"));
            }
            state.publications.subscribe()
        };
        // Any commit between registration and capture is included in capture;
        // the watch remains pending and a subsequent read safely finds no change.
        let mut observation = Observation {
            context,
            runtime: self.clone(),
            handle,
            selection,
            changed,
            resume,
            values: BTreeMap::new(),
            examined: None,
            fault: None,
            ended: false,
        };
        let initial = observation
            .capture(true)
            .expect("initial capture always publishes a result");
        Ok((observation, initial))
    }

    /// A finite coherent read does not register ongoing query interest.
    pub fn read_selection(&self, selection: &Selection) -> Result<Snapshot, Fault> {
        self.read_selection_with_context(None, selection)
    }

    pub(crate) fn read_selection_with_context(
        &self,
        context: Option<&CallContext>,
        selection: &Selection,
    ) -> Result<Snapshot, Fault> {
        self.validate_selection(selection)?;
        let mut state = self.state.lock().expect("session state is never poisoned");
        snapshot(self, &mut state, selection, context)
    }

    fn validate_selection(&self, selection: &Selection) -> Result<(), Fault> {
        if self.is_closed() {
            return Err(self.closure_fault());
        }
        selection.validate()?;
        if selection.scope != self.scope() {
            return Err(Fault::query("Owner incarnation does not match"));
        }
        for member in selection.members.values() {
            self.exports
                .get(&member.query.id)
                .ok_or_else(|| Fault::query("Query is not exported by this owner"))?
                .validate(member)?;
        }
        Ok(())
    }
}

/// Only immutable query values and faults are retained, never another document tree.
#[derive(Clone)]
enum Cached {
    Value(Value),
    Unavailable { fault: Fault, value: Option<Value> },
}
impl Cached {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Value(a), Self::Value(b)) => a.same(b),
            (Self::Unavailable { fault: a, .. }, Self::Unavailable { fault: b, .. }) => a == b,
            _ => false,
        }
    }
}
struct Evaluated {
    cached: Cached,
    // Materialized only when a value changed or a complete recovery is needed.
    content: Option<Content>,
}

impl Observation {
    pub fn handle(&self) -> Handle {
        self.handle
    }
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
    pub async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.changed.changed().await
    }
    pub fn poll(&mut self) -> Option<Publication> {
        self.capture(false)
    }

    fn capture(&mut self, opening: bool) -> Option<Publication> {
        if self.ended {
            return None;
        }
        if self.runtime.is_closed() {
            self.ended = true;
            return Some(Publication::Closed {
                handle: self.handle,
                reason: self.runtime.closure_fault(),
            });
        }
        let runtime = self.runtime.clone();
        let mut state = runtime
            .state
            .lock()
            .expect("session state is never poisoned");
        // Closure may have won while this capture waited for the owner lock.
        // Never evaluate private state interrupted by shutdown as current data.
        if runtime.is_closed() {
            self.ended = true;
            return Some(Publication::Closed {
                handle: self.handle,
                reason: runtime.closure_fault(),
            });
        }
        let position = state.publications.position();
        if !opening && self.examined == Some(position) {
            return None;
        }
        let live = if opening || self.fault.is_some() {
            None
        } else {
            self.resume
                .as_ref()
                .and_then(|resume| resume.publication)
                .and_then(|position| state.publications.streams_since(position))
        };
        let recovering = self.resume.is_some() && live.is_none();
        // All query results and document schemas are validated before any cursor
        // or comparison cache is changed. An optional fault remains a member result.
        let evaluated = match evaluate(
            &runtime,
            &mut state,
            &self.selection,
            &self.values,
            opening || recovering,
            self.context.as_ref(),
        ) {
            Ok(values) => values,
            Err(fault) => {
                self.examined = Some(position);
                if !opening && self.fault.as_ref() == Some(&fault) {
                    return None;
                }
                self.fault = Some(fault.clone());
                return Some(Publication::Fault {
                    handle: self.handle,
                    fault,
                });
            }
        };
        let mut documents = self
            .resume
            .as_ref()
            .map(|resume| resume.documents.clone())
            .unwrap_or_default();
        let publication = if self.resume.is_none() {
            let members = complete_contents(
                &mut state,
                &self.selection,
                evaluated
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.clone(),
                            value.content.clone().expect("initial result materialized"),
                        )
                    })
                    .collect(),
            );
            remember_documents(&members, &mut documents);
            Publication::Snapshot {
                handle: self.handle,
                snapshot: Snapshot { position, members },
            }
        } else if let Some(live) = live {
            let resume = self.resume.as_ref().unwrap();
            let mut members = BTreeMap::new();
            for (name, member) in &self.selection.members {
                if canonical(member) {
                    match state.view.sync(resume.documents.get(name), vec![]) {
                        ViewSync::Changes {
                            version, changes, ..
                        } => {
                            if !changes.is_empty() || !live.is_empty() {
                                documents.insert(name.clone(), version);
                                members.insert(
                                    name.clone(),
                                    Delta::Document {
                                        changes,
                                        streams: live.clone(),
                                    },
                                );
                            }
                        }
                        ViewSync::Snapshot { version, view, .. } => {
                            documents.insert(name.clone(), version.clone());
                            members.insert(
                                name.clone(),
                                Delta::Replace {
                                    content: Content::Document(Document {
                                        version,
                                        tree: view,
                                        streams: state.streams.values().cloned().collect(),
                                    }),
                                },
                            );
                        }
                    }
                } else if let Some(content) = &evaluated[name].content {
                    remember_document(name, content, &mut documents);
                    members.insert(
                        name.clone(),
                        Delta::Replace {
                            content: content.clone(),
                        },
                    );
                }
            }
            self.examined = Some(position);
            if members.is_empty() {
                self.values = evaluated
                    .into_iter()
                    .map(|(name, value)| (name, value.cached))
                    .collect();
                return None;
            }
            Publication::Update {
                handle: self.handle,
                update: Update {
                    from: resume
                        .publication
                        .expect("retained live history has a publication cursor"),
                    position,
                    members,
                },
            }
        } else {
            let resume = self.resume.as_ref().unwrap();
            let mut members = BTreeMap::new();
            for (name, member) in &self.selection.members {
                let recovery = if canonical(member) {
                    let streams = state.streams.values().cloned().collect();
                    match state.view.sync(resume.documents.get(name), streams) {
                        ViewSync::Changes {
                            version,
                            changes,
                            streams,
                        } => {
                            documents.insert(name.clone(), version);
                            Recovery::Document {
                                from: resume.documents[name].clone(),
                                changes,
                                streams,
                            }
                        }
                        ViewSync::Snapshot {
                            version,
                            view,
                            streams,
                        } => {
                            documents.insert(name.clone(), version.clone());
                            Recovery::Replace {
                                content: Content::Document(Document {
                                    version,
                                    tree: view,
                                    streams,
                                }),
                            }
                        }
                    }
                } else {
                    let content = evaluated[name]
                        .content
                        .clone()
                        .expect("recovery result materialized");
                    remember_document(name, &content, &mut documents);
                    Recovery::Replace { content }
                };
                members.insert(name.clone(), recovery);
            }
            Publication::Recovered {
                handle: self.handle,
                recovered: Recovered { position, members },
            }
        };
        self.fault = None;
        self.examined = Some(position);
        self.values = evaluated
            .into_iter()
            .map(|(name, value)| (name, value.cached))
            .collect();
        self.resume = Some(Resume {
            selection: self.selection.clone(),
            publication: Some(position),
            documents,
        });
        Some(publication)
    }
}

fn canonical(member: &Member) -> bool {
    member.query.id == CONVERSATION
}

fn remember_document(
    name: &str,
    content: &Content,
    documents: &mut BTreeMap<String, misa_proto::sync::Version>,
) {
    if let Content::Document(document) = content {
        documents.insert(name.into(), document.version.clone());
    } else {
        documents.remove(name);
    }
}
fn remember_documents(
    members: &BTreeMap<String, Content>,
    documents: &mut BTreeMap<String, misa_proto::sync::Version>,
) {
    for (name, content) in members {
        remember_document(name, content, documents);
    }
}

fn evaluate(
    runtime: &Runtime,
    state: &mut State,
    selection: &Selection,
    previous: &BTreeMap<String, Cached>,
    complete: bool,
    context: Option<&CallContext>,
) -> Result<BTreeMap<String, Evaluated>, Fault> {
    let mut values = BTreeMap::new();
    for (name, member) in &selection.members {
        if canonical(member) {
            continue;
        }
        let mut candidate = None;
        let result = if let Some(project) = runtime.restricted_exports.get(&member.query.id) {
            context
                .ok_or_else(|| Fault::query("Query requires an authenticated caller"))
                .and_then(|context| project(state, &member.query, context))
        } else {
            state
                .state
                .query(&member.query)
                .map_err(|fault| Fault::new(fault.code, fault.message))
        }
        .and_then(|value| {
            candidate = Some(value.clone());
            if let Some(Cached::Unavailable {
                fault,
                value: Some(old),
            }) = previous.get(name)
            {
                if old.same(&value) {
                    return Ok(Evaluated {
                        cached: Cached::Unavailable {
                            fault: fault.clone(),
                            value: Some(value),
                        },
                        content: complete.then(|| Content::Unavailable(fault.clone())),
                    });
                }
            }
            let cached = Cached::Value(value.clone());
            if !complete && previous.get(name).is_some_and(|old| old.same(&cached)) {
                return Ok(Evaluated {
                    cached,
                    content: None,
                });
            }
            let content = match &runtime.exports[&member.query.id].result {
                ResultContract::Data { schema } => {
                    schema
                        .validate(&value)
                        .map_err(|error| Fault::query(error.to_string()))?;
                    Content::Value(value)
                }
                ResultContract::Document {} => {
                    #[cfg(test)]
                    DOCUMENT_PARSES.with(|count| count.set(count.get() + 1));
                    let tree: misa_proto::Node =
                        crate::wire::parse(&value).map_err(Fault::query)?;
                    misa_proto::view::validate(&tree)
                        .map_err(|error| Fault::query(error.to_string()))?;
                    Content::Document(Document {
                        version: state.view.version.clone(),
                        tree,
                        streams: vec![],
                    })
                }
            };
            Ok(Evaluated {
                cached,
                content: Some(content),
            })
        });
        let evaluated = match result {
            Ok(value) => value,
            Err(fault) if member.optional => {
                let cached = Cached::Unavailable {
                    fault: fault.clone(),
                    value: candidate,
                };
                let changed = complete || !previous.get(name).is_some_and(|old| old.same(&cached));
                Evaluated {
                    cached,
                    content: changed.then_some(Content::Unavailable(fault)),
                }
            }
            Err(fault) => return Err(fault),
        };
        values.insert(name.clone(), evaluated);
    }
    Ok(values)
}

fn complete_contents(
    state: &mut State,
    selection: &Selection,
    mut values: BTreeMap<String, Content>,
) -> BTreeMap<String, Content> {
    for (name, member) in &selection.members {
        if canonical(member) {
            values.insert(
                name.clone(),
                Content::Document(Document {
                    version: state.view.version.clone(),
                    tree: state.view.tree.snapshot(),
                    streams: state.streams.values().cloned().collect(),
                }),
            );
        }
    }
    values
}

fn snapshot(
    runtime: &Runtime,
    state: &mut State,
    selection: &Selection,
    context: Option<&CallContext>,
) -> Result<Snapshot, Fault> {
    if runtime.is_closed() {
        return Err(runtime.closure_fault());
    }
    let evaluated = evaluate(runtime, state, selection, &BTreeMap::new(), true, context)?;
    let values = evaluated
        .into_iter()
        .map(|(name, value)| (name, value.content.expect("finite result materialized")))
        .collect();
    Ok(Snapshot {
        position: state.publications.position(),
        members: complete_contents(state, selection, values),
    })
}

#[cfg(test)]
mod closing_tests {
    use super::*;
    #[tokio::test]
    async fn closure_after_preflight_never_materializes_a_current_snapshot() {
        let runtime = Runtime::start(
            "closing-read",
            "Closing",
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::always("unused"),
            )),
            "scripted",
            "test",
            Value::Null,
        );
        let definition = runtime
            .query_exports()
            .into_iter()
            .find(|query| query.id == SUMMARY)
            .unwrap();
        let selection = Selection {
            scope: runtime.scope(),
            members: BTreeMap::from([("summary".into(), definition.member(vec![]).unwrap())]),
        };
        runtime.validate_selection(&selection).unwrap();
        // Model a read that passed preflight, then waited behind shutdown's owner
        // transaction. Materialization must recheck while holding that same lock.
        runtime.shutdown_complete().await;
        let mut state = runtime.state.lock().unwrap();
        assert_eq!(
            snapshot(&runtime, &mut state, &selection, None)
                .unwrap_err()
                .code,
            "closed_scope"
        );
    }
}
