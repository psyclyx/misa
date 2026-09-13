//! Shared attachment and resubscription state; transport only carries its messages.
use std::collections::BTreeMap;
use misa_proto::{ClientInfo, ClientMsg, SessionMsg, SessionInfo, Query, SubId, PROTOCOL_VERSION};
use misa_proto::sync::Version;

#[derive(Clone)]
struct Subscription { query: Query, version: Option<Version> }

pub struct Client {
    info: ClientInfo,
    session_id: String,
    session: Option<SessionInfo>,
    subscriptions: BTreeMap<SubId, Subscription>,
}
impl Client {
    pub fn new(info: ClientInfo, session: impl Into<String>) -> Self {
        Self { info, session_id: session.into(), session: None, subscriptions: BTreeMap::new() }
    }
    pub fn introduction(&mut self) -> Vec<ClientMsg> {
        self.session = None;
        let mut messages = vec![ClientMsg::Hello {version: PROTOCOL_VERSION, client: self.info.clone()}];
        if !self.session_id.is_empty() { messages.push(ClientMsg::Attach {session: self.session_id.clone()}); }
        messages
    }
    pub fn needs_attachment(&self) -> bool { !self.session_id.is_empty() }
    pub fn session(&self) -> Option<&SessionInfo> { self.session.as_ref() }
    pub fn subscribe(&mut self, id: SubId, query: Query, since: Option<Version>) -> ClientMsg {
        let since = if query.id == misa_proto::VIEW_QUERY { since } else { None };
        self.subscriptions.insert(id, Subscription {query: query.clone(), version: since.clone()});
        ClientMsg::Subscribe {id, query, since}
    }
    pub fn unsubscribe(&mut self, id: SubId) -> ClientMsg {
        self.subscriptions.remove(&id);
        ClientMsg::Unsubscribe {id}
    }
    pub fn resubscribe(&self) -> Vec<ClientMsg> {
        self.subscriptions.iter().map(|(id, subscription)| ClientMsg::Subscribe {
            id: *id, query: subscription.query.clone(), since: subscription.version.clone(),
        }).collect()
    }
    /// On a revision gap the driver sends this canonical request before reading further.
    pub fn reset_view(&mut self, id: SubId) -> Option<ClientMsg> {
        let subscription = self.subscriptions.get_mut(&id)?;
        subscription.version = None;
        Some(ClientMsg::Subscribe {id, query: subscription.query.clone(), since: None})
    }
    pub fn receive(&mut self, message: &SessionMsg) -> Result<(), String> {
        match message {
            SessionMsg::Welcome {version, session} => {
                if *version != PROTOCOL_VERSION { return Err("session protocol version does not match".into()); }
                if !session.id.is_empty() {
                    if session.id != self.session_id { return Err("daemon attached an unexpected session".into()); }
                    self.session = Some(session.clone());
                }
            }
            SessionMsg::View {id, version, ..} => {
                if let Some(subscription) = self.subscriptions.get_mut(id) { subscription.version = Some(version.clone()); }
            }
            SessionMsg::Changes {id, changes} => {
                if let Some(subscription) = self.subscriptions.get_mut(id) {
                    for change in changes {
                        if subscription.version.as_ref() != Some(&change.from)
                            || change.from.epoch != change.version.epoch || change.version.rev <= change.from.rev {
                            subscription.version = None;
                            return Err("subscription revision gap".into());
                        }
                        subscription.version = Some(change.version.clone());
                    }
                }
            }
            _ => {},
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::sync::{Change, ViewOp};
    use misa_proto::Node;
    #[test]
    fn reconnect_uses_the_last_received_cursor_and_never_replays_an_intent() {
        let mut client = Client::new(ClientInfo::new("test", "1"), "demo");
        assert_eq!(client.introduction().len(), 2);
        let initial = Version {epoch: "run".into(), rev: 7};
        client.subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY), Some(initial.clone()));
        let next = Version {rev: 8, ..initial.clone()};
        client.receive(&SessionMsg::Changes {id: SubId(1), changes: vec![Change {
            from: initial, version: next.clone(), ops: vec![ViewOp::Insert {parent: "session".into(), before: None, node: Node::section("child").id("child")}],
        }]}).unwrap();
        client.introduction();
        assert!(matches!(&client.resubscribe()[0], ClientMsg::Subscribe {since: Some(version), ..} if version == &next));
        client.unsubscribe(SubId(1));
        assert!(client.resubscribe().is_empty());
    }
    #[test]
    fn a_gap_requests_canonical_state_without_advancing_the_cursor() {
        let mut client = Client::new(ClientInfo::new("test", "1"), "demo");
        client.subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY), None);
        let from = Version {epoch: "run".into(), rev: 4};
        assert!(client.receive(&SessionMsg::Changes {id: SubId(1), changes: vec![Change {from: from.clone(), version: Version {rev: 5, ..from}, ops: vec![]}]}).is_err());
        assert!(matches!(client.reset_view(SubId(1)), Some(ClientMsg::Subscribe {since: None, ..})));
    }
}
