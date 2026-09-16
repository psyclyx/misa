//! The protocol reads a running session through this application boundary.
use crate::{Runtime, Reading, Emission};
use misa_proto::{Fault, Query, SessionInfo};
use misa_proto::view::Choice;
use misa_proto::wire::{Intent, RequestContext};
use misa_proto::sync::{Version, ViewSync, Stream};
use tokio::sync::{broadcast, mpsc, watch};

impl misa_protocol::owner::Owner for Runtime {
    fn read(&self, context: &misa_protocol::invocation::CallContext, selection: &misa_proto::observation::Selection) -> Result<misa_proto::observation::Snapshot, Fault> {
        self.read_selection_with_context(Some(context), selection)
    }
    fn observe(self: std::sync::Arc<Self>, context: misa_protocol::invocation::CallContext,
        handle: misa_proto::observation::Handle, selection: misa_proto::observation::Selection, resume: Option<misa_proto::observation::Resume>,
    ) -> Result<(Box<dyn misa_protocol::owner::Observation>, misa_proto::observation::Publication), Fault> {
        let (observation, publication) = Runtime::observe_with_context(&self, Some(context), handle, selection, resume)?;
        Ok((Box::new(observation), publication))
    }
}

impl misa_protocol::Session for Runtime {
    fn read(&self, query: &Query) -> Result<Reading, Fault> { self.read(query) }
    fn rev(&self) -> u64 { self.rev() }
    fn info(&self) -> SessionInfo { self.info() }
    fn intent_from(&self, intent: Intent, context: Option<RequestContext>) -> Vec<Fault> { self.intent_from(intent, context) }
    fn complete(&self, source: &str, prefix: &str, limit: Option<u32>) -> Result<(Vec<Choice>, bool), Fault> { self.complete(source, prefix, limit) }
    fn sync(&self, since: Option<&Version>) -> (ViewSync, u64) { self.sync_with_events(since) }
    fn changes(&self, since: Option<&Version>) -> ViewSync { self.changes(since) }
    fn streams(&self) -> Vec<Stream> { self.streams() }
    fn watch_rev(&self) -> watch::Receiver<u64> { self.watch_rev() }
    fn subscribe_events(&self) -> broadcast::Receiver<Emission> { self.subscribe_events() }
    fn subscribe_replies(&self, connection: u64) -> mpsc::Receiver<Emission> { self.replies.register(connection) }
    fn attached(&self, connection: u64, client: misa_proto::ClientInfo) {
        let mut clients = self.clients.lock().unwrap();
        clients.insert(connection, client);
        self.publish_clients(&clients);
    }
    fn detached(&self, connection: u64) {
        self.replies.remove(connection);
        let mut clients = self.clients.lock().unwrap();
        if clients.remove(&connection).is_some() { self.publish_clients(&clients); }
    }
}

impl Runtime {
    fn publish_clients(&self, clients: &std::collections::BTreeMap<u64, misa_proto::ClientInfo>) {
        use misa_value::Value;
        self.dispatch(misa_reframe::Event::new("clients/changed").with("clients", Value::list(clients.values().map(|client|
            Value::map([("name", Value::str(&client.name)), ("version", Value::str(&client.version))])
        ).collect::<Vec<_>>())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[tokio::test]
    async fn status_lists_client_names_and_versions_and_tracks_disconnects() {
        let runtime = Runtime::start("clients", "Clients", None,
            Arc::new(misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::always("done"))),
            "scripted", "test", misa_value::Value::Null);
        let mut first = misa_protocol::Server::new(runtime.clone());
        first.handle(misa_proto::ClientMsg::Hello {version: misa_proto::PROTOCOL_VERSION, client: misa_proto::ClientInfo::new("terminal", "1.2")});
        let mut second = misa_protocol::Server::new(runtime.clone());
        second.handle(misa_proto::ClientMsg::Hello {version: misa_proto::PROTOCOL_VERSION, client: misa_proto::ClientInfo::new("phone", "3.4")});
        assert!(runtime.intent(Intent::Command {name: "status".into(), args: misa_value::Value::Null}).is_empty());
        let text = || misa_render::to_plain(&misa_render::render(&runtime.view().unwrap(), &misa_render::Theme::plain(), 100));
        assert!(text().contains("terminal 1.2") && text().contains("phone 3.4"));
        drop(second);
        assert!(text().contains("terminal 1.2") && !text().contains("phone 3.4"));
    }
}
