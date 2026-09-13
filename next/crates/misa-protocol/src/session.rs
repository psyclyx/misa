//! Application state required by a connection's protocol state machine.
use misa_proto::{Fault, Node, Query, SessionInfo};
use misa_proto::view::Choice;
use misa_proto::wire::{ClientInfo, Intent, RequestContext, SessionEvent};
use misa_proto::sync::{Stream, Version, ViewSync};
use misa_value::Value;
use tokio::sync::{broadcast, watch};

#[derive(Clone, Debug)]
pub enum Reading { View(Node), Data(Value) }

#[derive(Clone, Debug)]
pub struct Emission {
    pub recipient: Option<u64>,
    pub seq: u64,
    pub event: SessionEvent,
}

/// Implemented by the application. The protocol neither constructs nor owns its kernel.
pub trait Session: Send + Sync {
    fn read(&self, query: &Query) -> Result<Reading, Fault>;
    fn rev(&self) -> u64;
    fn info(&self) -> SessionInfo;
    fn intent_from(&self, intent: Intent, context: Option<RequestContext>) -> Vec<Fault>;
    fn complete(&self, source: &str, prefix: &str, limit: Option<u32>) -> Result<(Vec<Choice>, bool), Fault>;
    fn sync(&self, since: Option<&Version>) -> ViewSync;
    fn changes(&self, since: Option<&Version>) -> ViewSync;
    fn streams(&self) -> Vec<Stream>;
    fn watch_rev(&self) -> watch::Receiver<u64>;
    fn subscribe_events(&self) -> broadcast::Receiver<Emission>;
    /// The connection, rather than a client-supplied name, is its lifecycle identity.
    fn attached(&self, _connection: u64, _client: ClientInfo) {}
    fn detached(&self, _connection: u64) {}
}
