//! The vocabulary that crosses between an agent session and a client.
//!
//! This crate is what both ends agree on, and it is deliberately the only such
//! thing. It holds no policy, performs no IO, and draws nothing.
//!
//! # The three stages
//!
//! The previous system named the stages `data → presentation data → renderer` and
//! only ever reached the second one in the same process as the first. Splitting
//! the agent across a network makes the last stage unavoidable, so this crate
//! states all three as a contract:
//!
//! - **Facts** are the session's own business and never appear here.
//! - **A view tree** ([`view`]) is what a session tells a client about. It says
//!   *what is here*: roles, structure, typed fields, stable node identities, and
//!   the named actions a node offers. It has no field that can express a colour, a
//!   font, a size, a coordinate, or a cell — not by policy but by vocabulary.
//! - **A surface** is the client's. Cells, DOM, or pixels; theming, wrapping,
//!   scrolling, focus, and animation.
//!
//! # The load-bearing rule
//!
//! > A session may not decide appearance, and a client may not invent agent
//! > state.
//!
//! The first half is enforced by construction: the vocabulary has nothing to
//! decide appearance *with*. The second half is enforced by the shape of
//! [`wire::Intent`]: a client can submit text, resolve a node's own action, run a
//! named command, or cancel — and nothing else. A client cannot describe an agent
//! effect, because no message type carries one.
//!
//! # The corollary that matters in practice
//!
//! Because the vocabulary has no appearance, it must carry the *facts an
//! appearance decision needs*. A session does not say "this thinking block is
//! collapsed"; it says the block is streaming and how large it is, and
//! [`view::Kind::Collapsible`] says that the node has a short form and a long
//! form. Whether the long form is showing is the client's business, and two
//! clients with different tastes both get to be right.

pub mod blob;
pub mod chunk;
pub mod frame;
pub mod view;
pub mod sync;
pub mod wire;

pub use blob::{BlobMsg, BlobReply, MAX_BLOB_BYTES, MAX_BLOB_FRAME};
pub use frame::{FrameError, decode, encode};
pub use view::Node;
pub use wire::{
    ClientInfo, ClientMsg, Fault, Intent, Level, Query, SessionEvent, SessionInfo,
    Pairing, SessionMsg, SubId, Ticket,
};

/// The version both ends announce. A peer that announces another major version is
/// refused before it can act, rather than being partially understood.
pub const PROTOCOL_VERSION: u16 = 2;

/// Client to session: the whole client vocabulary. One bidirectional stream per
/// connection carries it.
pub const ALPN_SESSION: &[u8] = b"/misa/session/2";

/// Bulk content by hash. A view node that carries an image names a blob here
/// rather than inlining bytes, so a transcript stays small and a client fetches
/// what it can actually display.
pub const ALPN_BLOB: &[u8] = b"/misa/blob/0";

/// A session runner to a kernel. Reserved: the shipped daemon hosts its sessions
/// in-process, and this is the seam a separately hosted session would use.
pub const ALPN_KERNEL: &[u8] = b"/misa/kernel/0";

/// Legacy bounded framing limit, retained for non-session envelopes.
/// Session values use [`chunk`] and have no whole-message ceiling.
pub const MAX_CONTROL_FRAME: usize = 8 * 1024 * 1024;

/// The canonical session document query.
pub const VIEW_QUERY: &str = "session.view";

/// Names of the shipped completion sources, shared by sessions and clients.
pub mod completion {
    pub const MODELS: &str = "models";
    pub const EFFORT: &str = "effort";
    pub const COMMANDS: &str = "commands";
    pub const PROVIDERS: &str = "providers";
    pub const CONVERSATIONS: &str = "conversations";
    pub const CONVERSATIONS_QUERY: &str = "session.conversations";
    pub const MODELS_QUERY: &str = "completion.models";
    pub const EFFORT_QUERY: &str = "completion.effort";
    pub const COMMANDS_QUERY: &str = "completion.commands";
    pub const PROVIDERS_QUERY: &str = "completion.providers";
}
