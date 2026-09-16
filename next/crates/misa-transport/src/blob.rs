//! Blobs over iroh: the server, the client, and the framing between them.
//!
//! This is the other half of [`misa_proto::blob`]. That module says what the messages
//! are; this one moves them, and it is the same split as the session connection: the
//! vocabulary is testable without a socket, and the socket is the only thing here.
//!
//! # Shape of the connection
//!
//! One bidirectional stream, opened by the client, carrying [`BlobMsg`] up and
//! [`BlobReply`] down. Strictly one answer per request, in order: the server never speaks
//! first. That is what lets the whole loop be a `read → answer → write` with no second
//! task and no correlation ids — a blob request is not an intent, it has no id, and it
//! cannot be answered out of order because there is never more than one in flight.
//!
//! # Who may connect
//!
//! The same [`Roster`] the session handler consults, checked at accept time, before a
//! single byte is read. A daemon's blob store holds the images of every session on it, so
//! "may this peer talk to me at all" has to be asked here as well as there — and asking
//! it before a message arrives means a refused peer learns nothing about what is in the
//! store, not even which hashes exist.

use std::sync::Arc;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use misa_proto::ALPN_BLOB;
use misa_proto::blob::{BlobMsg, BlobReply, MAX_BLOB_FRAME};
use misa_proto::frame::{Decoder, decode, encode_within};
use misa_proto::view::BlobRef;
use crate::admission::Admission;
use tracing::{debug, warn};

/// How much is read at a time.
///
/// A blob at the bound is a megabyte of memory in flight per read, not thirty-two: the
/// decoder grows only by what has arrived, and it refuses a declared length over the
/// frame bound before allocating for it.
const READ_CHUNK: usize = 1024 * 1024;

/// Storage supplied by the daemon. Client builds contain no kernel implementation.
pub trait BlobStore: Send + Sync {
    fn get(&self, hash: &str) -> Option<Vec<u8>>;
    fn media(&self, hash: &str) -> Option<String>;
    fn has(&self, hash: &str) -> bool;
    fn store(&self, bytes: Vec<u8>, media: Option<&str>) -> Result<BlobRef, String>;
}

/// Serves blobs from a store to admitted peers.
pub struct Handler {
    pub blobs: Arc<dyn BlobStore>,
    pub admission: Arc<Admission>,
}

impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `ProtocolHandler` requires `Debug`. A store has nothing useful to print and a
        // list of hashes would be worse than nothing.
        f.write_str("misa blob handler")
    }
}

impl ProtocolHandler for Handler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer = connection.remote_id();
        if !self.admission.admits(&peer.to_string()) {
            // Closed rather than answered: a peer that is not admitted should not be able
            // to tell a session's blob store from a daemon with no sessions at all.
            warn!(peer = %peer, "refused a blob connection: not on the admission roster");
            return Ok(());
        }
        debug!(peer = %peer, "a peer opened the blob connection");
        let (send, recv) = connection.accept_bi().await?;
        let peer_id = peer.to_string();
        let mut changed = self.admission.watch();
        let conversation = converse(self.blobs.clone(), self.admission.clone(), peer_id.clone(), send, recv);
        tokio::pin!(conversation);
        loop {
            if !self.admission.admits(&peer_id) {
                connection.close(0u32.into(), b"admission revoked");
                return Ok(());
            }
            tokio::select! {
                result = &mut conversation => {
                    if let Err(error) = result {
                        warn!(peer = %peer, error = %error, "a blob connection ended badly");
                        connection.close(0u32.into(), b"blob request refused");
                    }
                    break;
                }
                _ = changed.changed() => {}
            }
        }
        connection.closed().await;
        Ok(())
    }
}

/// Answer requests until the client stops asking.
async fn converse(blobs: Arc<dyn BlobStore>, admission: Arc<Admission>, peer: String, mut send: SendStream, mut recv: RecvStream) -> Result<(), String> {
    let mut decoder = Decoder::with_limit(MAX_BLOB_FRAME);
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        let Some(payload) = read_frame(&mut recv, &mut decoder, &mut buffer).await? else {
            return Ok(());
        };
        if !admission.admits(&peer) { return Err("admission revoked".into()); }
        let reply = match decode::<BlobMsg>(&payload) {
            Ok(message) => match message.acceptable() {
                Err(fault) => BlobReply::Refused { fault },
                Ok(()) => answer(blobs.as_ref(), message),
            },
            Err(error) => BlobReply::refused(format!("that is not a blob request: {error}")),
        };
        write_reply(&mut send, &reply).await?;
    }
}

/// What one request means.
///
/// Pure, and the only place a request becomes an answer: the store is asked, and whatever
/// it says — including a refusal for being full — is what the client is told.
fn answer(blobs: &dyn BlobStore, message: BlobMsg) -> BlobReply {
    match message {
        BlobMsg::Get { hash } => match blobs.get(&hash) {
            // The media type is read before the hash is moved: it is the store's memory of
            // what these bytes are, and a client that had to guess would guess wrong for
            // exactly the files worth displaying.
            Some(bytes) => {
                let media = blobs.media(&hash);
                BlobReply::Bytes { hash, media, bytes }
            }
            None => BlobReply::Missing { hash },
        },
        BlobMsg::Have { hashes } => BlobReply::Have {
            hashes: hashes.into_iter().filter(|hash| blobs.has(hash)).collect(),
        },
        BlobMsg::Put { bytes, media } => match blobs.store(bytes, media.as_deref()) {
            Ok(blob) => BlobReply::Stored { hash: blob.hash, len: blob.len, media: blob.media },
            Err(message) => BlobReply::refused(message),
        },
    }
}

async fn read_frame(
    recv: &mut RecvStream,
    decoder: &mut Decoder,
    buffer: &mut [u8],
) -> Result<Option<Vec<u8>>, String> {
    loop {
        match decoder.next() {
            Some(Ok(payload)) => return Ok(Some(payload)),
            Some(Err(error)) => return Err(error.to_string()),
            None => match recv.read(buffer).await {
                Ok(None) => return Ok(None),
                Ok(Some(bytes)) => decoder.push(&buffer[..bytes]).map_err(|err| err.to_string())?,
                Err(err) => return Err(err.to_string()),
            },
        }
    }
}

async fn write_reply(send: &mut SendStream, reply: &BlobReply) -> Result<(), String> {
    let frame = encode_within(reply, MAX_BLOB_FRAME).map_err(|err| err.to_string())?;
    send.write_all(&frame).await.map_err(|err| err.to_string())
}

/// A client of a daemon's blob store.
///
/// Deliberately three methods, one per message. A client that needs more than fetch, check
/// and upload is a client that needs a new message, which is the conversation to have
/// rather than a way around it.
pub struct Client {
    send: SendStream,
    recv: RecvStream,
    decoder: Decoder,
    buffer: Vec<u8>,
}

/// Bytes that arrived, with what they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    pub hash: String,
    pub media: Option<String>,
    pub bytes: Vec<u8>,
}

impl Client {
    /// Connect to a daemon's blob store.
    ///
    /// The `address` is the same one a session ticket carries: the store is served by the
    /// endpoint that serves the session, not by a service of its own.
    pub async fn connect(endpoint: &Endpoint, address: EndpointAddr) -> Result<Client, String> {
        let connection = endpoint
            .connect(address, ALPN_BLOB)
            .await
            .map_err(|err| format!("could not reach the blob store: {err}"))?;
        let (send, recv) = connection
            .open_bi()
            .await
            .map_err(|err| format!("could not open a blob stream: {err}"))?;
        Ok(Client { send, recv, decoder: Decoder::with_limit(MAX_BLOB_FRAME), buffer: vec![0u8; READ_CHUNK] })
    }

    async fn ask(&mut self, message: &BlobMsg) -> Result<BlobReply, String> {
        let frame = encode_within(message, MAX_BLOB_FRAME).map_err(|err| err.to_string())?;
        self.send.write_all(&frame).await.map_err(|err| err.to_string())?;
        let Some(payload) = read_frame(&mut self.recv, &mut self.decoder, &mut self.buffer).await? else {
            return Err("the blob store closed the connection".to_string());
        };
        match decode::<BlobReply>(&payload).map_err(|err| err.to_string())? {
            BlobReply::Refused { fault } => Err(format!("{}: {}", fault.code, fault.message)),
            other => Ok(other),
        }
    }

    /// Fetch one blob.
    ///
    /// `Ok(None)` is a blob the store does not have, which is not an error: a transcript
    /// outlives the bytes it points at only if somebody deleted them, and a client that
    /// cannot show an image has to be able to say so rather than fail.
    pub async fn get(&mut self, hash: &str) -> Result<Option<Blob>, String> {
        match self.ask(&BlobMsg::Get { hash: hash.to_string() }).await? {
            BlobReply::Bytes { hash: received, media, bytes } => {
                if received != hash || blake3::hash(&bytes).to_hex().to_string() != hash {
                    return Err("The blob response does not match the requested content hash".into());
                }
                Ok(Some(Blob { hash: received, media, bytes }))
            },
            BlobReply::Missing { .. } => Ok(None),
            other => Err(format!("expected bytes, got a `{}`", other.name())),
        }
    }

    /// Which of these hashes the store already has.
    ///
    /// Asked before an upload: an attachment that has been sent before costs no bytes.
    pub async fn have(&mut self, hashes: &[String]) -> Result<Vec<String>, String> {
        match self.ask(&BlobMsg::Have { hashes: hashes.to_vec() }).await? {
            BlobReply::Have { hashes } => Ok(hashes),
            other => Err(format!("expected a list of hashes, got a `{}`", other.name())),
        }
    }

    /// Upload bytes, and get back the name they now have.
    ///
    /// What a client does with a file it can read and the daemon cannot: the bytes travel,
    /// the content hash is computed on the other side, and the `BlobRef` that comes back is
    /// what an attachment names.
    pub async fn put(&mut self, bytes: Vec<u8>, media: Option<&str>) -> Result<BlobRef, String> {
        let message = BlobMsg::Put { bytes, media: media.map(str::to_string) };
        match self.ask(&message).await? {
            BlobReply::Stored { hash, len, media } => Ok(BlobRef { hash, len, media }),
            other => Err(format!("expected a stored blob, got a `{}`", other.name())),
        }
    }

    /// Upload bytes that the store may already hold.
    ///
    /// The round trip a client actually wants: ask, and send the bytes only if the answer
    /// was no. Two messages for one intent, which is why it lives here rather than in
    /// every frontend.
    pub async fn share(&mut self, bytes: Vec<u8>, media: Option<&str>) -> Result<BlobRef, String> {
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let len = bytes.len() as u64;
        if !self.have(std::slice::from_ref(&hash)).await?.is_empty() {
            return Ok(BlobRef { hash, len, media: media.map(str::to_string) });
        }
        self.put(bytes, media).await
    }
}

/// A blob store on another endpoint, with the connection it takes to ask.
///
/// A frontend wants a blob once per picture and then never again — the bytes are
/// content-addressed and a browser caches them forever — so what this holds is a connection
/// opened when it is first wanted rather than at startup, and reopened once when it has gone.
/// One reconnect is the whole retry policy, and it is enough because the alternative to a live
/// connection is not a person waiting: it is the next image fetch, which can try again.
pub struct Store {
    endpoint: Endpoint,
    address: std::sync::Mutex<EndpointAddr>,
    connection: tokio::sync::Mutex<Option<Client>>,
}

impl Store {
    pub fn new(endpoint: Endpoint, address: EndpointAddr) -> Arc<Store> {
        Arc::new(Store { endpoint, address: std::sync::Mutex::new(address), connection: tokio::sync::Mutex::new(None) })
    }

    pub fn refresh_address(&self, address: EndpointAddr) -> Result<(), String> {
        let mut current = self.address.lock().expect("blob address poisoned");
        if current.id != address.id { return Err("Routing hints name a different daemon".into()); }
        *current = address;
        Ok(())
    }
    fn address(&self) -> EndpointAddr {
        self.address.lock().expect("blob address poisoned").clone()
    }

    /// One request, over the connection this store keeps, reconnecting once if it has gone.
    async fn ask(&self, message: BlobMsg) -> Result<BlobReply, String> {
        let mut guard = self.connection.lock().await;
        // Keep an in-flight stream out of the reusable slot. Cancellation can
        // interrupt either framing direction; dropping the request must drop
        // that stream instead of handing a partial reply to the next caller.
        let mut connection = match guard.take() {
            Some(connection) => connection,
            None => Client::connect(&self.endpoint, self.address()).await?,
        };
        let answer = connection.ask(&message).await;
        match answer {
            Ok(reply) => { *guard = Some(connection); Ok(reply) },
            Err(first) => {
                // A connection that has gone is not a store that has no bytes, and reconnecting
                // once is what tells the two apart.
                *guard = None;
                let mut fresh = Client::connect(&self.endpoint, self.address())
                    .await
                    .map_err(|err| format!("{first}; reconnecting: {err}"))?;
                let answer = fresh.ask(&message).await;
                if answer.is_ok() { *guard = Some(fresh); }
                answer
            }
        }
    }

    /// The bytes a hash names, or `None` when the store does not have them.
    pub async fn get(&self, hash: &str) -> Result<Option<Blob>, String> {
        match self.ask(BlobMsg::Get { hash: hash.to_string() }).await? {
            BlobReply::Bytes { hash: received, media, bytes } => {
                if received != hash || blake3::hash(&bytes).to_hex().to_string() != hash {
                    return Err("The blob response does not match the requested content hash".into());
                }
                Ok(Some(Blob { hash: received, media, bytes }))
            },
            BlobReply::Missing { .. } => Ok(None),
            other => Err(format!("expected bytes, got a `{}`", other.name())),
        }
    }

    /// Put bytes here, and get back the name they now have.
    ///
    /// The round trip a client that read a file actually wants: ask what the store has, and
    /// send the bytes only if the answer was no. An attachment that has been uploaded before
    /// costs a question and nothing else.
    pub async fn share(&self, bytes: Vec<u8>, media: Option<&str>) -> Result<BlobRef, String> {
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let len = bytes.len() as u64;
        match self.ask(BlobMsg::Have { hashes: vec![hash.clone()] }).await? {
            BlobReply::Have { hashes } if !hashes.is_empty() => {
                return Ok(BlobRef { hash, len, media: media.map(str::to_string) });
            }
            BlobReply::Have { .. } => {}
            other => return Err(format!("expected a list of hashes, got a `{}`", other.name())),
        }
        match self.ask(BlobMsg::Put { bytes, media: media.map(str::to_string) }).await? {
            BlobReply::Stored { hash, len, media } => Ok(BlobRef { hash, len, media }),
            other => Err(format!("expected a stored blob, got a `{}`", other.name())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::Fixture;
    use misa_kernel::{Provider, ScriptedProvider};
    use std::time::Duration;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];

    #[derive(Debug)]
    struct PartialReply {
        attempts: Arc<std::sync::atomic::AtomicUsize>,
        partial: Arc<tokio::sync::Notify>,
    }
    impl ProtocolHandler for PartialReply {
        async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
            let (mut send, mut recv) = connection.accept_bi().await?;
            let mut decoder = Decoder::with_limit(MAX_BLOB_FRAME);
            let mut buffer = vec![0; 1024];
            let payload = read_frame(&mut recv, &mut decoder, &mut buffer).await.unwrap().unwrap();
            let BlobMsg::Get { hash } = decode(&payload).unwrap() else { panic!("expected fetch") };
            let reply = encode_within(&BlobReply::Missing { hash }, MAX_BLOB_FRAME).unwrap();
            if self.attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                send.write_all(&reply[..reply.len() - 1]).await.unwrap();
                self.partial.notify_one();
                connection.closed().await;
            } else {
                send.write_all(&reply).await.unwrap();
                connection.closed().await;
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancelling_a_partial_reply_discards_the_stream_before_the_next_fetch() {
        let server = crate::iroh::bind(None, false).await.unwrap();
        let client = crate::iroh::bind(None, false).await.unwrap();
        let partial = Arc::new(tokio::sync::Notify::new());
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let router = iroh::protocol::Router::builder(server.clone()).accept(ALPN_BLOB, PartialReply { attempts: attempts.clone(), partial: partial.clone() }).spawn();
        let store = Store::new(client.clone(), server.addr());
        let first_store = store.clone();
        let first = tokio::spawn(async move { first_store.get(&"0".repeat(64)).await });
        within("partial response", partial.notified()).await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        assert!(within("fresh fetch", store.get(&"1".repeat(64))).await.unwrap().is_none());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
        drop(store);
        client.close().await;
        router.shutdown().await.unwrap();
    }

    /// A bound on every await in these tests.
    ///
    /// A test that hangs is a test that reports nothing, and a transport that never
    /// answers is exactly the failure mode worth catching. Five seconds is several orders
    /// of magnitude more than a loopback round trip.
    async fn within<F: std::future::Future>(what: &str, future: F) -> F::Output {
        match tokio::time::timeout(Duration::from_secs(5), future).await {
            Ok(value) => value,
            Err(_) => panic!("{what} did not finish"),
        }
    }

    fn provider() -> Arc<dyn Provider> {
        ScriptedProvider::always("hello")
    }

    #[tokio::test]
    async fn a_store_uploads_bytes_the_daemon_may_not_have_and_names_them_by_content() {
        // What a frontend with a file to send wants: put the bytes, get the name back, and pay
        // nothing the second time the same bytes are sent.
        let fixture = Fixture::start(Admission::open(), provider()).await;
        let store = Store::new(fixture.client.clone(), fixture.address.clone());

        let stored = within("an upload", store.share(PNG.to_vec(), Some("image/png"))).await.expect("an upload");
        assert!(fixture.blobs.has(&stored.hash), "the daemon does not have the bytes");
        assert_eq!(stored.len, PNG.len() as u64);

        // The same bytes again: the store asked first and the answer was yes, so nothing was
        // sent — and the name it comes back with is the same name, because it is a hash.
        let again = within("a second upload", store.share(PNG.to_vec(), Some("image/png"))).await.expect("an upload");
        assert_eq!(again.hash, stored.hash);

        let fetched = within("a fetch", store.get(&stored.hash)).await.expect("a fetch").expect("the blob");
        assert_eq!(fetched.bytes, PNG);

        // And bytes the daemon has never seen are named too, which is the whole of an upload.
        let other = within("a different upload", store.share(b"not a picture".to_vec(), None)).await;
        assert_ne!(other.expect("an upload").hash, stored.hash);
    }

    #[tokio::test]
    async fn a_store_keeps_the_connection_it_wants_and_reopens_one_that_has_gone() {
        // What a frontend wants: ask for a picture, then ask for the next one over the same
        // connection, and be told plainly when the bytes are not there.
        let fixture = Fixture::start(Admission::open(), provider()).await;
        let stored = fixture.blobs.put(PNG, None).expect("a blob");
        let store = Store::new(fixture.client.clone(), fixture.address.clone());

        let fetched = within("a fetch", store.get(&stored.hash)).await.expect("a fetch").expect("the blob");
        assert_eq!(fetched.bytes, PNG);
        assert_eq!(fetched.media.as_deref(), Some("image/png"));

        // The same connection, asked again.
        let again = within("a second fetch", store.get(&stored.hash)).await.expect("a fetch").expect("the blob");
        assert_eq!(again.bytes, PNG);

        // A blob nobody has is an answer, not a failure: a transcript outlives the bytes only
        // when somebody deleted them.
        let missing = "0".repeat(64);
        assert!(within("a miss", store.get(&missing)).await.expect("an answer").is_none());

        // And a name that is not a hash never reaches the other end at all.
        assert!(within("a refusal", store.get("../../etc/shadow")).await.is_err());
    }

    #[tokio::test]
    async fn bytes_make_the_round_trip_over_a_real_endpoint() {
        let fixture = Fixture::start(Admission::open(), provider()).await;
        let mut client = within("connecting", Client::connect(&fixture.client, fixture.address.clone()))
            .await
            .expect("a client");

        // Nothing is there yet, and that is an answer rather than a failure. The hash is
        // shaped like a hash — 64 lowercase hex characters — and names no blob.
        let missing = "0".repeat(64);
        assert!(within("a fetch", client.get(&missing)).await.expect("an answer").is_none());

        let stored = within("an upload", client.put(PNG.to_vec(), None)).await.expect("an upload");
        assert_eq!(stored.media.as_deref(), Some("image/png"), "the server sniffed the bytes");
        assert_eq!(stored.len, PNG.len() as u64);
        assert!(fixture.blobs.has(&stored.hash), "the bytes are in the daemon's store");

        let fetched = within("a fetch", client.get(&stored.hash)).await.expect("a fetch").expect("the blob");
        assert_eq!(fetched.bytes, PNG);
        assert_eq!(fetched.media.as_deref(), Some("image/png"));
        assert_eq!(fetched.hash, stored.hash);

        // A hash the store holds and a hash it has never seen, in one question.
        let asked = within("a have", client.have(&[stored.hash.clone(), missing.clone()]))
            .await
            .expect("a have");
        assert_eq!(asked, vec![stored.hash.clone()]);

        // And the same bytes again cost nothing, which is what content addressing buys: the
        // client asks first and sends nothing.
        let again = within("a share", client.share(PNG.to_vec(), None)).await.expect("a share");
        assert_eq!(again.hash, stored.hash);
        fixture.stop().await;
    }

    #[tokio::test]
    async fn a_request_that_names_something_that_is_not_a_hash_is_refused_without_being_served() {
        let fixture = Fixture::start(Admission::open(), provider()).await;
        let mut client = within("connecting", Client::connect(&fixture.client, fixture.address.clone()))
            .await
            .expect("a client");
        let error = within("a fetch", client.get("../../etc/shadow")).await.expect_err("a refusal");
        assert!(error.contains("blob.hash"), "{error}");
        // The connection survives a refused request, so a client can correct itself.
        let stored = within("an upload", client.put(b"fine".to_vec(), None)).await.expect("an upload");
        assert!(within("a fetch", client.get(&stored.hash)).await.expect("a fetch").is_some());
        fixture.stop().await;
    }

    #[tokio::test]
    async fn a_peer_that_is_not_admitted_is_closed_before_it_can_ask_anything() {
        let fixture = Fixture::start(Admission::listed([]), provider()).await;
        // The store holds something, so a peer that could reach it would learn that a hash
        // exists. It cannot: the connection is closed at accept, before a byte is read.
        fixture.blobs.put(PNG, None).unwrap();
        let client = within("connecting", Client::connect(&fixture.client, fixture.address.clone())).await;
        match client {
            // Refused at the stream, which is where iroh reports a server that hung up.
            Err(_) => {}
            Ok(mut client) => {
                let asked = within("a fetch", client.get(&"0".repeat(64))).await;
                assert!(asked.is_err(), "a peer that was refused got an answer: {asked:?}");
            }
        }
        fixture.stop().await;
    }

    #[tokio::test]
    async fn one_connection_answers_many_requests_in_order() {
        // A client fetches what it can draw as it draws it, so several requests share one
        // connection. Answers must come back in the order they were asked for, which is what
        // having no correlation ids costs and what the round trip proves.
        let fixture = Fixture::start(Admission::open(), provider()).await;
        let first = fixture.blobs.put(b"one", None).unwrap();
        let second = fixture.blobs.put(b"two", None).unwrap();
        let mut client = within("connecting", Client::connect(&fixture.client, fixture.address.clone()))
            .await
            .expect("a client");
        for _ in 0..4 {
            let asked = within(
                "a pair of fetches",
                async {
                    let a = client.get(&first.hash).await?;
                    let b = client.get(&second.hash).await?;
                    Ok::<_, String>((a, b))
                },
            )
            .await
            .expect("two answers");
            assert_eq!(asked.0.expect("the first blob").bytes, b"one");
            assert_eq!(asked.1.expect("the second blob").bytes, b"two");
        }
        fixture.stop().await;
    }
}
