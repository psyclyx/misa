// A policy plugin, as a guest sees it: four functions, all of them data.
//
// The shapes are generated from `../policy.wit`, so this file is also the check on it: a
// record with a field the wit does not have, a variant arm that is not there, or a signature
// that does not match stops the build with the wit's own name for the mismatch.
//
// What is worth noticing while reading it: nothing here can reach a credential, a socket, a
// file, or a colour. The only things this program may do are return patches, ask for effects
// the host's interpreter accepts, and return view nodes — because those are the only types in
// the world.
wit_bindgen::generate!({
    path: "../policy.wit",
    world: "policy",
});

use exports::misa::policy::policy_api::{
    Descriptor, Effect, Event, Fault, Guest, Op, OptionValue, Patch, QueryRequest,
    QueryDefinition, QuerySource, ReadContract, Presentation, PresentationVariant,
    CommandDefinition, ActionBinding,
};

struct Shell;

impl Guest for Shell {
    /// The one call that registers nothing at runtime: what this plugin handles, answers, and
    /// may ask for, told to the host once, at install.
    fn describe() -> Descriptor {
        Descriptor {
            tools: vec![],
            id: "policy.guest".to_string(),
            version: "0.2.0".to_string(),
            // Commands use explicitly installed plugin events.
            events: vec![
                "intent/prompt".to_string(),
                "plugin.policy.guest.refresh".to_string(),
                "intent/cancel".to_string(),
            ],
            // What its views offer. A tree that offers anything else is refused when it is
            // presented.
            commands: vec![CommandDefinition {
                id: "policy.guest.refresh".into(), event: "plugin.policy.guest.refresh".into(),
                input: r#"{"type":"record","fields":{"confirm":{"schema":{"type":"bool"}}}}"#.into(),
            }],
            bindings: vec![ActionBinding { id: "policy.guest.refresh".into(), command: "policy.guest.refresh".into(),
                bound: r#"{"confirm":true}"#.into(), inputs: "{}".into(),
            }],
            queries: vec![
                QueryDefinition {
                    id: "policy.guest.turns".into(), contract: "policy.guest.turns@1".into(), arguments: vec![],
                    output: r#"{"kind":"data","schema":{"type":"record","fields":{"answered":{"schema":{"type":"string"}}}}}"#.into(),
                    source: QuerySource::Derived(vec![]),
                },
                QueryDefinition {
                    id: "policy.guest.state".into(), contract: "policy.guest.state@1".into(), arguments: vec![],
                    output: r#"{"kind":"data","schema":{"type":"record","fields":{},"allow_unknown":true}}"#.into(),
                    source: QuerySource::Read(ReadContract {
                        roots: vec!["guest".into()],
                        schema: r#"{"type":"record","fields":{"guest":{"optional":true,"schema":{"type":"record","fields":{},"allow_unknown":true}}}}"#.into(),
                    }),
                },
                QueryDefinition {
                    id: "policy.guest.copy".into(), contract: "policy.guest.copy@1".into(), arguments: vec![],
                    output: r#"{"kind":"data","schema":{"type":"record","fields":{},"allow_unknown":true}}"#.into(),
                    source: QuerySource::Derived(vec![QueryRequest { id: "policy.guest.state".into(), args: vec![] }]),
                },
                QueryDefinition {
                    id: "policy.guest.document".into(), contract: "policy.guest.document@1".into(), arguments: vec![], output: r#"{"kind":"document"}"#.into(),
                    source: QuerySource::Read(ReadContract { roots: vec!["guest".into()],
                        schema: r#"{"type":"record","fields":{"guest":{"optional":true,"schema":{"type":"record","fields":{},"allow_unknown":true}}}}"#.into(),
                    }),
                },
            ],
            presentations: vec![Presentation { id: "main".into(), title: "Guest".into(), variants: vec![PresentationVariant {
                id: "semantic".into(), query: QueryRequest { id: "policy.guest.document".into(), args: vec![] }, requirements: vec![],
            }] }],
            // An effect the session's interpreter accepts, and whose data is json: wire.event
            // is the one kind a plugin may not ask for, and the host refuses it at install.
            effects: vec!["kernel.log.append".to_string()],
            // The root this plugin keeps its own state in — the same name the patch below
            // writes into, which is what makes the declaration worth having.
            roots: vec!["guest".to_string()],
        }
    }

    fn configure(_options: Vec<OptionValue>) -> Result<(), Fault> {
        Ok(())
    }

    /// One event in, patches and effects out — validated by the host *before* anything
    /// commits, which is why an unknown event is a fault and not a `panic!`.
    fn handle(event: Event, db: String) -> Result<(Vec<Patch>, Vec<Effect>), Fault> {
        // The host validated the command before dispatching its installed event.
        if event.kind == "plugin.policy.guest.refresh" {
            return Ok((
                vec![Patch {
                    path: "guest.acted".to_string(),
                    op: Op::Set("true".to_string()),
                }],
                vec![],
            ));
        }
        if event.kind != "intent/prompt" {
            return Err(Fault {
                code: "policy.guest.unexpected".to_string(),
                message: format!("`{}` is not an event this plugin handles", event.kind),
                event: Some(event.kind),
            });
        }
        Ok((
            vec![Patch {
                path: "guest.turns".to_string(),
                op: Op::Set(format!("{{\"seen\":{}}}", db.len())),
            }],
            vec![Effect {
                kind: "kernel.log.append".to_string(),
                data: Some(
                    "{\"conversation\":\"guest\",\"kind\":\"note\",\"data\":\"seen a prompt\"}"
                        .to_string(),
                ),
            }],
        ))
    }

    /// A query's answer, as json. `previous` is a hint: the same inputs must produce the same
    /// answer with or without it.
    ///
    /// `policy.guest.state` echoes only its declared read data, making isolation observable.
    fn query(
        request: QueryRequest,
        inputs: Vec<String>,
        read_data: Option<String>,
        _previous: Option<String>,
    ) -> Result<String, Fault> {
        if request.id == "policy.guest.document" { return document(read_data.unwrap()); }
        if request.id == "policy.guest.state" {
            return Ok(read_data.unwrap());
        }
        if read_data.is_some() {
            return Err(Fault { code: "guest.unexpected-data".into(), message: "derived query received database data".into(), event: None });
        }
        if request.id == "policy.guest.copy" {
            return Ok(inputs[0].clone());
        }
        Ok(format!("{{ \"answered\": \"{}\" }}", request.id))
    }

}

fn document(db: String) -> Result<String, Fault> {
    if db.contains("\"spin\"") { loop { std::hint::black_box(1); } }
    if db.contains("\"refuse\"") {
        return Err(Fault { code: "policy.guest.no-view".into(), message: "this plugin cannot draw that".into(), event: None });
    }
    Ok(format!(r#"{{"id":"guest","role":"guest.panel","kind":{{"shape":"section"}},"actions":[{{"id":"policy.guest.refresh","label":"Refresh"}}],"children":[{{"id":"guest.summary","role":"guest.summary","kind":{{"shape":"status","text":"{} bytes of state, drawn for semantic"}}}}]}}"#, db.len()))
}

export!(Shell);
