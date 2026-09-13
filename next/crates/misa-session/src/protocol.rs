//! The protocol reads a running session through this application boundary.
use crate::{Runtime, Reading, Emission};
use misa_proto::{Fault, Query, SessionInfo};
use misa_proto::view::Choice;
use misa_proto::wire::{Intent, RequestContext};
use misa_proto::sync::{Version, ViewSync, Stream};
use tokio::sync::{broadcast, watch};

impl misa_protocol::Session for Runtime {
    fn read(&self, query: &Query) -> Result<Reading, Fault> { self.read(query) }
    fn rev(&self) -> u64 { self.rev() }
    fn info(&self) -> SessionInfo { self.info() }
    fn intent_from(&self, intent: Intent, context: Option<RequestContext>) -> Vec<Fault> { self.intent_from(intent, context) }
    fn complete(&self, source: &str, prefix: &str, limit: Option<u32>) -> Result<(Vec<Choice>, bool), Fault> { self.complete(source, prefix, limit) }
    fn sync(&self, since: Option<&Version>) -> ViewSync { self.sync(since) }
    fn changes(&self, since: Option<&Version>) -> ViewSync { self.changes(since) }
    fn streams(&self) -> Vec<Stream> { self.streams() }
    fn watch_rev(&self) -> watch::Receiver<u64> { self.watch_rev() }
    fn subscribe_events(&self) -> broadcast::Receiver<Emission> { self.subscribe_events() }
}
