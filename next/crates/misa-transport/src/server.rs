//! One endpoint, two protocols.
//!
//! A daemon serves sessions and blobs from the same iroh endpoint, and this is the only
//! place that says so. It is a module rather than two `serve` functions because two
//! functions would be two ways to build a router, and a daemon that mounted one of them
//! would silently be a daemon whose tickets point at a node that does not answer for
//! images.
//!
//! # Why one endpoint
//!
//! A ticket is `misa:<node>:<session>`: one string, one node, one session. If blobs were
//! served by a second endpoint, every ticket would have to carry a second address, every
//! client would have to keep two connections in step, and the admission decision would
//! have to be made twice by two pieces of code that could disagree. Serving both from one
//! endpoint makes "can this peer fetch this session's images" the same question as "can
//! this peer talk to this session", which is the question a [`Roster`] answers.

use std::sync::Arc;

// `::iroh` rather than `iroh`, because this file also names our own transport module
// `iroh`. A name that can mean two things is a name to spell out.
use ::iroh::protocol::Router;
use ::iroh::Endpoint;
#[cfg(test)]
use ::iroh::EndpointAddr;
#[cfg(test)]
use misa_kernel::Blobs;
use misa_proto::{ALPN_BLOB, ALPN_SESSION};
use crate::admission::Admission;

use crate::iroh::{self, Sessions};
use crate::blob;

/// Start serving: sessions on one ALPN, their bytes on another, one admission decision.
pub fn serve(
    endpoint: Endpoint,
    sessions: Arc<Sessions>,
    blobs: Arc<dyn blob::BlobStore>,
    admission: Arc<Admission>,
) -> Router {
    Router::builder(endpoint)
        .accept(ALPN_SESSION, iroh::Handler { sessions, admission: admission.clone() })
        .accept(ALPN_BLOB, blob::Handler { blobs, admission })
        .spawn()
}

/// A daemon endpoint with one session and one blob store, and where to reach it.
///
/// What every transport test needs and none of them should build by hand: two endpoints —
/// one serving, one dialing, because connecting to yourself is not a thing iroh does — and
/// a session composed over an in-memory kernel.
#[cfg(test)]
pub(crate) struct Fixture {
    pub server: Endpoint,
    pub client: Endpoint,
    pub address: EndpointAddr,
    pub sessions: Arc<Sessions>,
    pub blobs: Arc<Blobs>,
    pub router: Router,
    /// Kept so the daemon can be taken away and brought back with the same door.
    admission: Arc<Admission>,
}

#[cfg(test)]
impl Fixture {
    pub(crate) async fn start(admission: Admission, provider: Arc<dyn misa_kernel::Provider>) -> Fixture {
        let server = iroh::bind(None, false).await.expect("a server endpoint");
        let client = iroh::bind(None, false).await.expect("a client endpoint");
        let sessions = Sessions::new();
        sessions.insert(misa_session::Runtime::start(
            "demo",
            "a demo session",
            None,
            Arc::new(misa_kernel::LocalKernel::new(provider)),
            "scripted",
            "scripted-1",
            misa_value::Value::Null,
        ));
        let blobs = Arc::new(Blobs::in_memory());
        let address = iroh::address_of(&iroh::node_of(&server)).expect("an address");
        let admission = Arc::new(admission);
        let router = serve(server.clone(), sessions.clone(), Arc::new(KernelBlobs(blobs.clone())), admission.clone());
        Fixture { server, client, address, sessions, blobs, router, admission }
    }

    /// Take the daemon away without taking its identity away: what a restart is.
    ///
    /// The endpoint stays bound — its key *is* the identity a ticket names — while the router
    /// that was answering on it goes, which drops every connection it was serving. `shutdown`
    /// cannot be used for this: it closes the endpoint too, and then there is nothing to come
    /// back to.
    pub(crate) fn stop_serving(&mut self) {
        let back = serve(self.server.clone(), self.sessions.clone(), Arc::new(KernelBlobs(self.blobs.clone())), self.admission.clone());
        let dead = std::mem::replace(&mut self.router, back);
        drop(dead);
    }

    pub(crate) async fn stop(self) {
        self.router.shutdown().await.ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob as blob_client;
    use crate::iroh::Client;
    use misa_kernel::{Provider, ScriptedProvider, Turn};
    use misa_proto::wire::{Intent, SessionMsg, SubId};
    use misa_proto::{ClientInfo, Query};
    use misa_value::Value;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];

    /// Every await in these tests is bounded, because a transport that never answers is a
    /// test suite that hangs instead of reporting.
    async fn within<F: std::future::Future>(what: &str, future: F) -> F::Output {
        match tokio::time::timeout(std::time::Duration::from_secs(10), future).await {
            Ok(value) => value,
            Err(_) => panic!("{what} did not finish"),
        }
    }

    fn scripted() -> Arc<dyn Provider> {
        ScriptedProvider::new([
            Turn::call("echo", Value::str("hello"), Turn::say("all done")),
            Turn::say("all done"),
        ])
    }

    fn client_info() -> ClientInfo {
        ClientInfo::new("test-client", "0.1.0")
    }

    #[tokio::test]
    async fn a_large_canonical_view_crosses_a_real_endpoint() {
        let fixture = Fixture::start(Admission::open(), scripted()).await;
        let runtime = misa_session::Runtime::start(
            "demo", "large", None, Arc::new(misa_kernel::LocalKernel::new(scripted())),
            "scripted", "x".repeat(misa_proto::MAX_CONTROL_FRAME + 1024), Value::Null,
        );
        fixture.sessions.insert(runtime.clone());
        let expected = runtime.view().unwrap();
        let mut client = within(
            "attaching",
            Client::connect(&fixture.client, fixture.address.clone(), client_info(), "demo"),
        ).await.unwrap();
        within("subscribing", client.subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY)))
            .await.unwrap();
        match within("large view", client.next()).await.unwrap().unwrap() {
            SessionMsg::View { view, .. } => assert_eq!(view, expected),
            other => panic!("expected the whole view, got {other:?}"),
        }
        fixture.stop().await;
    }

    /// The whole point of the transport, end to end: a real endpoint, a real client, a
    /// prompt, and a transcript that changes.
    ///
    /// This is the test that would have caught the second copy of the session loop: that
    /// copy answered a subscription once and never refreshed it, so a client over a real
    /// connection saw an empty transcript and then silence.
    #[tokio::test]
    async fn a_client_attaches_prompts_and_sees_the_transcript_change() {
        let fixture = Fixture::start(Admission::open(), scripted()).await;
        let mut client = within(
            "attaching",
            Client::connect(&fixture.client, fixture.address.clone(), client_info(), "demo"),
        )
        .await
        .expect("a connection");
        // `connect` has already read past the endpoint's greeting: the first thing a caller
        // sees from an attached connection is the session's own answer to a subscription.
        assert_eq!(client.session().as_ref().map(|session| session.id.as_str()), Some("demo"));
        within("subscribing", client.subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY)))
            .await
            .expect("a subscription");
        let first = within("a view", client.next()).await.expect("a message").expect("a view");
        let mut state = misa_proto::sync::ClientView::default();
        state.receive(&first).unwrap();
        let before = state.canonical().unwrap();
        assert!(
            misa_proto::view::find(&before, "transcript").is_some(),
            "the first view has no transcript"
        );

        within(
            "sending",
            client.intent(1, Intent::Prompt { text: "what is this".into(), attachments: Vec::new() }),
        )
        .await
        .expect("an intent");

        // The session answers with an acknowledgement and then with views: one where the
        // prompt has been written, and one the reply has landed in. The second is the part
        // that used to never arrive, because the network path had its own copy of the
        // session loop and that copy never watched the revision.
        let mut saw_ack = false;
        let mut saw_prompt = false;
        let mut saw_reply = false;
        for _ in 0..60 {
            let Some(message) = within("a message", client.next()).await.expect("a message") else {
                break;
            };
            saw_ack |= matches!(message, SessionMsg::Ack { .. });
            if state.receive(&message).unwrap() {
                if let Some(view) = state.canonical() {
                    let text =
                        misa_render::to_plain(&misa_render::render(&view, &misa_render::Theme::plain(), 100));
                    saw_prompt |= text.contains("what is this");
                    saw_reply |= text.contains("all done");
                    if saw_prompt && saw_reply {
                        break;
                    }
                }
            }
        }
        assert!(saw_ack, "the intent was never acknowledged");
        assert!(saw_prompt, "the transcript never showed the prompt");
        assert!(saw_reply, "the transcript never showed the reply");
        fixture.stop().await;
    }

    #[tokio::test]
    async fn one_ticket_reaches_both_the_session_and_its_bytes() {
        let fixture = Fixture::start(Admission::open(), scripted()).await;
        // The bytes a view would point at, put where the daemon serves them.
        let stored = fixture.blobs.put(PNG, None).expect("a blob");

        // The same address, the same endpoint: no second ticket and no second address.
        let mut session = within(
            "attaching",
            Client::connect(&fixture.client, fixture.address.clone(), client_info(), "demo"),
        )
        .await
        .expect("a session connection");
        assert!(session.session().is_some(), "the connection is not attached to anything");
        let mut blobs = within("connecting", blob_client::Client::connect(&fixture.client, fixture.address.clone()))
            .await
            .expect("a blob connection");
        let fetched = within("a fetch", blobs.get(&stored.hash)).await.expect("a fetch").expect("the blob");
        assert_eq!(fetched.bytes, PNG);

        // And the session connection is still attached, which is what "in step" means here:
        // two connections, one identity, neither disturbing the other.
        within("a subscription", session.subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY)))
            .await
            .expect("a subscription");
        assert!(matches!(
            within("a view", session.next()).await.expect("a message"),
            Some(SessionMsg::View { .. })
        ));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn an_empty_roster_admits_nobody_on_either_protocol() {
        let fixture = Fixture::start(Admission::listed([]), scripted()).await;
        fixture.blobs.put(PNG, None).expect("a blob");

        let session = within(
            "attaching",
            Client::connect(&fixture.client, fixture.address.clone(), client_info(), "demo"),
        )
        .await;
        // Either the connection is refused outright or it is closed the moment it is used;
        // what must not happen is a session arriving.
        if let Ok(mut client) = session {
            match within("a read", client.next()).await {
                Err(_) => {}
                Ok(None) => {}
                Ok(Some(message)) => panic!("a refused peer was handed {message:?}"),
            }
        }

        let blobs = within("connecting", blob_client::Client::connect(&fixture.client, fixture.address.clone())).await;
        if let Ok(mut client) = blobs {
            assert!(
                within("a fetch", client.get(&"0".repeat(64))).await.is_err(),
                "a refused peer got an answer from the blob store"
            );
        }
        fixture.stop().await;
    }

    #[tokio::test]
    async fn the_ticket_a_daemon_prints_is_the_one_that_reaches_its_session() {
        // The line a daemon prints is the whole of what a person carries: one node, one
        // session. It has to parse back into somewhere to dial and something to ask for,
        // because that string is the entire interface to a running session.
        let fixture = Fixture::start(Admission::open(), scripted()).await;
        assert_eq!(fixture.sessions.ids(), vec!["demo".to_string()]);

        let ticket = iroh::ticket(&fixture.server, "demo");
        let parsed: misa_proto::Ticket = ticket.to_string().parse().expect("a ticket is text");
        assert_eq!(parsed.session, "demo");
        let address = iroh::address_of(&parsed.node).expect("a ticket says where to dial");
        assert_eq!(address.id, fixture.address.id);

        let client = within(
            "attaching",
            Client::connect(&fixture.client, address, client_info(), &parsed.session),
        )
        .await
        .expect("the printed ticket reaches the session");
        assert_eq!(client.session().as_ref().map(|session| session.id.as_str()), Some("demo"));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn attaching_to_a_session_the_daemon_does_not_have_says_so() {
        let fixture = Fixture::start(Admission::open(), scripted()).await;
        let refused = within(
            "attaching",
            Client::connect(&fixture.client, fixture.address.clone(), client_info(), "nope"),
        )
        .await;
        match refused {
            Err(message) => assert!(message.contains("nope"), "{message}"),
            // A session nobody has must not be handed out, and if the connection is not
            // refused outright then reading from it has to fail.
            Ok(mut client) => assert!(
                within("a read", client.next()).await.is_err(),
                "a session nobody has was handed out"
            ),
        }
        fixture.stop().await;
    }

    /// Attach to the fixture's session and subscribe to its view, which is the state every
    /// client in these tests is in before anything happens.
    async fn attached(endpoint: &::iroh::Endpoint, fixture: &Fixture) -> Client {
        let mut client = within(
            "attaching",
            Client::connect(endpoint, fixture.address.clone(), client_info(), "demo"),
        )
        .await
        .expect("a connection");
        within(
            "subscribing",
            client.subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY)),
        )
        .await
        .expect("a subscription");
        client
    }

    /// Read views until the transcript says the thing, and hand back what it says.
    async fn settled(client: &mut Client, looking_for: &str) -> String {
        let mut state = misa_proto::sync::ClientView::default();
        settled_with(client, &mut state, looking_for).await
    }

    async fn settled_with(client: &mut Client, state: &mut misa_proto::sync::ClientView, looking_for: &str) -> String {
        for _ in 0..80 {
            let Some(message) = within("a message", client.next()).await.expect("a message") else {
                break;
            };
            state.receive(&message).unwrap();
            if let Some(view) = state.canonical() {
                let text = misa_render::to_plain(&misa_render::render(
                    &view,
                    &misa_render::Theme::plain(),
                    100,
                ));
                if text.contains(looking_for) && text.contains(" · idle · ") {
                    return text;
                }
            }
        }
        panic!("the transcript never said `{looking_for}`");
    }

    /// Two clients on one session, which is a claim the architecture has been making and
    /// nothing has tested.
    ///
    /// What it pins is *convergence* rather than delivery. A subscription's value is the whole
    /// current state, so a client that attaches late, or that was looking away when something
    /// changed, ends up with the same transcript as the one that was there. There is no replay
    /// protocol to get wrong — and the second client here is the proof: it hears about a
    /// prompt it did not send, and reads the same words.
    #[tokio::test]
    async fn two_clients_on_one_session_converge_on_the_same_transcript() {
        let fixture = Fixture::start(Admission::open(), scripted()).await;
        // Two dialing endpoints, because iroh does not connect an endpoint to itself and
        // because two clients are two peers.
        let other = iroh::bind(None, false).await.expect("a second client endpoint");
        let mut first = attached(&fixture.client, &fixture).await;
        let mut second = attached(&other, &fixture).await;

        // One of them speaks. The other one did not, and is not told that it happened —
        // only that the state changed.
        within(
            "sending",
            first.intent(1, Intent::Prompt { text: "who is there".into(), attachments: Vec::new() }),
        )
        .await
        .expect("an intent");

        let first_text = within("the first client", settled(&mut first, "all done")).await;
        let second_text = within("the second client", settled(&mut second, "all done")).await;
        assert!(first_text.contains("who is there"), "{first_text}");
        assert!(second_text.contains("who is there"), "{second_text}");
        // A client's own window and scroll are its own, so what is compared is the words the
        // transcript holds, which is what a subscription promises to keep in step.
        assert_eq!(first_text, second_text, "\n{first_text}\n---\n{second_text}");
        fixture.stop().await;
    }

    /// A connection is not the session. When the daemon goes away and comes back — the same
    /// endpoint, the same session, a new socket — a client finds its own way back.
    ///
    /// Nothing above this layer has to know it happened, which is the point: a frontend sees a
    /// stream of views, not a stream of connections. What it takes for that to be correct is a
    /// subscription that converges, and this asserts it by comparing the transcript across the
    /// drop: same words, including the turn that happened before it.
    #[tokio::test]
    async fn a_client_finds_its_way_back_when_the_connection_drops() {
        let mut fixture = Fixture::start(Admission::open(), scripted()).await;
        let mut client = attached(&fixture.client, &fixture).await;
        within(
            "sending",
            client.intent(1, Intent::Prompt { text: "before the drop".into(), attachments: Vec::new() }),
        )
        .await
        .expect("an intent");
        let mut state = misa_proto::sync::ClientView::default();
        let before = within("the first transcript", settled_with(&mut client, &mut state, "all done")).await;
        assert!(before.contains("before the drop"), "{before}");
        assert_eq!(client.reconnects(), 0, "the client reconnected before anything dropped");

        // The daemon goes away and comes back, with the same identity and the same session.
        fixture.stop_serving();

        // Reading is what notices: nothing tells the client, it finds out when the connection
        // it is reading from is gone — which is after whatever was already in flight arrives,
        // since a drop does not un-send a message — and it repairs that by itself.
        let after = within("a transcript after the drop", async {
            loop {
                let text = settled_with(&mut client, &mut state, "all done").await;
                if client.reconnects() >= 1 {
                    return text;
                }
            }
        })
        .await;
        // The re-subscribed value is the whole current state, so it still holds what happened
        // before the drop. It is not compared whole: the session was in the middle of a turn
        // when the connection went, and a turn that kept going is not a lost transcript.
        assert!(after.contains("before the drop"), "{after}");
        assert!(after.contains("all done"), "{after}");
        assert_eq!(client.reconnects(), 1, "the client re-established more than once");
        fixture.stop().await;
    }
}

#[cfg(test)]
struct KernelBlobs(Arc<Blobs>);
#[cfg(test)]
impl blob::BlobStore for KernelBlobs {
    fn get(&self, hash: &str) -> Option<Vec<u8>> { self.0.get(hash) }
    fn media(&self, hash: &str) -> Option<String> { self.0.media(hash) }
    fn has(&self, hash: &str) -> bool { self.0.has(hash) }
    fn store(&self, bytes: Vec<u8>, media: Option<&str>) -> Result<misa_proto::view::BlobRef, String> { self.0.store(bytes, media) }
}
