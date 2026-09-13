//! The wasm plugin host: a component, as handlers the loop can run.
//!
//! `wit/policy.wit` is the contract and this is the side that holds it. A plugin is a
//! component that declares what it handles, answers queries, and — for every event it
//! declared — returns patches and effects. The host's whole job is the translation between
//! that and what [`misa_reframe`] already does: an [`Event`] in, a [`Handler`] out, patches
//! applied through [`Tx`], and effects queued for the interpreter to accept or refuse.
//!
//! # What this crate deliberately is not
//!
//! It knows nothing about a turn, a provider, a session, or a client. It does not decide what
//! an effect *means* — the session's interpreter does, and a plugin asking for something the
//! composition does not accept is a fault either at install (because it declared it) or in the
//! transaction (because it tried it). That is why this crate depends on the loop and the view
//! vocabulary and on nothing above them.
//!
//! # The four boundaries, and where each is enforced
//!
//! - **A plugin declares; it does not register.** [`Plugin::describe`] is called once, at
//!   load, and [`Plugin::validate`] compares those declarations against the composition
//!   before anything runs. Nothing a plugin does later adds a handler.
//! - **A plugin never sees a secret.** Nothing in the world can express one: the data that
//!   crosses is json, and a credential is a slot the session's own effects name.
//! - **A plugin cannot present.** It returns view *nodes* — roles, structure, typed fields —
//!   and the host validates them with [`misa_proto::view::validate`], the same function a
//!   session's own view is held to. There is no type here that can express a colour, a size,
//!   or a position.
//! - **A plugin may fail.** A guest fault, a json payload that does not decode, a path that
//!   is not one, a tree deeper than the protocol allows: each is a [`Fault`] that rolls the
//!   plugin's transaction back and is reported. A trap is the same, one level down — a
//!   component that panics is a component whose call returns an error, not a session that
//!   dies.
//!
//! # Why a plugin's call is behind a lock
//!
//! A wasmtime [`Store`](wasmtime::Store) is `Send` and not `Sync`, and a [`Handler`] has to be
//! both. One call at a time per plugin is also the *contract*, not a limitation: a guest that
//! could re-enter itself would need a stack discipline nobody has designed, and a plugin's
//! handler is a pure function of one event.

mod handler;
mod host;

pub use handler::{PLUGIN_PRIORITY, PluginHandler};
pub use host::{Descriptor, Patch, Plugin, PluginFault};
