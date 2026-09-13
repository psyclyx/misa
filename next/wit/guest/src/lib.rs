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
    Action, Descriptor, Effect, Event, Fault, Guest, Node, Op, OptionValue, Patch, QueryRequest, ViewTree,
};

struct Shell;

impl Guest for Shell {
    /// The one call that registers nothing at runtime: what this plugin handles, answers, and
    /// may ask for, told to the host once, at install.
    fn describe() -> Descriptor {
        Descriptor {
            id: "policy.guest".to_string(),
            version: "0.1.0".to_string(),
            // Three kinds, and every one of them is the loop's own vocabulary. `intent/action`
            // is how this plugin is acted on: an affordance in a tree it presented comes back as
            // that event, and nothing on this side has to know where the session filed it.
            events: vec![
                "intent/prompt".to_string(),
                "intent/action".to_string(),
                "intent/cancel".to_string(),
            ],
            // What its views offer. A tree that offers anything else is refused when it is
            // presented.
            actions: vec!["refresh".to_string()],
            queries: vec!["policy.guest.turns".to_string(), "policy.guest.state".to_string()],
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
        // An affordance from a tree this plugin presented comes back as the loop's own action
        // event, with the action id, the node, and whatever fields the client sent.
        if event.kind == "intent/action" {
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
    /// `policy.guest.state` answers with the database it was handed, verbatim: a guest has no
    /// parser here, and handing back what it was given is the one thing that shows a caller
    /// what this plugin could see.
    fn query(
        request: QueryRequest,
        _inputs: Vec<String>,
        db: String,
        _previous: Option<String>,
    ) -> Result<String, Fault> {
        if request.id == "policy.guest.state" {
            return Ok(db);
        }
        Ok(format!("{{ \"answered\": \"{}\" }}", request.id))
    }

    /// A tree, as a flat list: a plugin cannot present, so it says what the nodes *are*.
    ///
    /// The ids are the plugin's own — a client remembers which nodes it opened by id — and the
    /// action is the plugin's own too: the session routes it back here because the node it sits
    /// on is one this plugin offered. The database is checked for `spin`, which is a switch a
    /// test flips to prove that a runaway guest is stopped by its budget rather than by anybody
    /// noticing.
    fn view(db: String) -> Result<ViewTree, Fault> {
        if db.contains("\"spin\"") {
            let mut turns: u64 = 0;
            loop {
                turns = turns.wrapping_add(1);
                std::hint::black_box(turns);
            }
        }
        if db.contains("\"refuse\"") {
            return Err(Fault {
                code: "policy.guest.no-view".to_string(),
                message: "this plugin cannot draw that".to_string(),
                event: None,
            });
        }
        Ok(ViewTree {
            nodes: vec![
                Node {
                    id: "guest".to_string(),
                    role: "guest.panel".to_string(),
                    kind: "section".to_string(),
                    data: None,
                    parent: None,
                    actions: vec![Action {
                        id: "refresh".to_string(),
                        label: Some("Refresh".to_string()),
                        on_submit: false,
                        args: None,
                    }],
                    state: None,
                },
                Node {
                    id: "guest.summary".to_string(),
                    role: "guest.summary".to_string(),
                    kind: "status".to_string(),
                    // A semantic report of the state the session supplied.
                    data: Some(format!(
                        "{{\"text\":\"{} bytes of state, drawn for {}\"}}",
                        db.len(),
                        "semantic",
                    )),
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
