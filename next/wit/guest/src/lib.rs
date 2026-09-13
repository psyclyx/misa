// A policy plugin, as a guest sees it: five functions, all of them data.
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
    Descriptor, Effect, Event, Fault, Guest, Node, Op, OptionValue, Patch, QueryRequest, ViewTree,
};

struct Shell;

impl Guest for Shell {
    /// The one call that registers nothing at runtime: what this plugin handles, answers, and
    /// may ask for, told to the host once, at install.
    fn describe() -> Descriptor {
        Descriptor {
            id: "policy.guest".to_string(),
            version: "0.1.0".to_string(),
            // Two kinds: the one it handles, and one it declares so that it is *called* for
            // it and can refuse. A plugin is only ever asked about what it declared, which is
            // what makes the second kind the only way to reach the fault path.
            events: vec!["intent/prompt".to_string(), "intent/cancel".to_string()],
            queries: vec!["policy.guest.turns".to_string()],
            // An effect the session's interpreter accepts, and whose data is json: wire.event
            // is the one kind a plugin may not ask for, and the host refuses it at install.
            effects: vec!["kernel.log.append".to_string()],
        }
    }

    fn configure(_options: Vec<OptionValue>) -> Result<(), Fault> {
        Ok(())
    }

    /// One event in, patches and effects out — validated by the host *before* anything
    /// commits, which is why an unknown event is a fault and not a `panic!`.
    fn handle(event: Event, db: String) -> Result<(Vec<Patch>, Vec<Effect>), Fault> {
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
    fn query(
        request: QueryRequest,
        _inputs: Vec<String>,
        _db: String,
        _previous: Option<String>,
    ) -> Result<String, Fault> {
        Ok(format!("{{ \"answered\": \"{}\" }}", request.id))
    }

    /// A tree, as a flat list: a plugin cannot present, so it says what the nodes *are*.
    fn view(_role: String, _capabilities: String, _db: String, _window: u32) -> Result<ViewTree, Fault> {
        Ok(ViewTree {
            nodes: vec![
                Node {
                    id: "guest".to_string(),
                    role: "guest.panel".to_string(),
                    kind: "section".to_string(),
                    data: None,
                    parent: None,
                    actions: Vec::new(),
                    state: None,
                },
                Node {
                    id: "guest.note".to_string(),
                    role: "guest.note".to_string(),
                    kind: "text".to_string(),
                    data: Some("{\"spans\":[]}".to_string()),
                    parent: Some(0),
                    actions: Vec::new(),
                    state: None,
                },
            ],
            root: 0,
        })
    }
}

export!(Shell);
