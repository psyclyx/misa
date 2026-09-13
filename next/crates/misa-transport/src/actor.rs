//! One owner for connection IO. Cancelling a frontend wait never cancels a write or handshake.
use super::WireClient;
use iroh::{Endpoint, EndpointAddr};
use misa_proto::{ClientInfo, Intent, Query, SessionInfo, SessionMsg, SubId, sync::Version};
use std::sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}};
use tokio::sync::{mpsc, oneshot};

const CAPACITY: usize = 64;

enum Request {
    Subscribe { id: SubId, query: Query, since: Option<Version> },
    Intent { id: u64, intent: Intent },
}
struct Command { request: Request, reply: oneshot::Sender<Result<(), String>> }
#[derive(Default)]
struct Status {
    session: Mutex<Option<SessionInfo>>,
    reconnects: AtomicU32,
}
impl Status {
    fn refresh(&self, wire: &impl Wire) {
        *self.session.lock().expect("connection status is never poisoned") = wire.session();
        self.reconnects.store(wire.reconnects(), Ordering::Release);
    }
}

/// The frontend's handle. Its only message read is a cancellation-safe bounded receive.
pub struct Client {
    commands: mpsc::Sender<Command>,
    incoming: mpsc::Receiver<Result<SessionMsg, String>>,
    status: Arc<Status>,
    task: tokio::task::AbortHandle,
}
impl Drop for Client {
    fn drop(&mut self) { self.task.abort(); }
}
impl Client {
    pub async fn connect(endpoint: &Endpoint, address: EndpointAddr, info: ClientInfo, session: &str) -> Result<Self, String> {
        let wire = WireClient::connect(endpoint, address, info, session).await?;
        Ok(Self::spawn(wire))
    }
    fn spawn(wire: impl Wire + 'static) -> Self {
        let (commands, receive_commands) = mpsc::channel(CAPACITY);
        let (send_messages, incoming) = mpsc::channel(CAPACITY);
        let status = Arc::new(Status::default());
        status.refresh(&wire);
        let task = tokio::spawn(run(wire, receive_commands, send_messages, status.clone())).abort_handle();
        Self { commands, incoming, status, task }
    }
    pub async fn pair(endpoint: &Endpoint, address: EndpointAddr, code: &str, label: &str) -> Result<String, String> {
        WireClient::pair(endpoint, address, code, label).await
    }
    pub fn session(&self) -> Option<SessionInfo> {
        self.status.session.lock().expect("connection status is never poisoned").clone()
    }
    pub fn reconnects(&self) -> u32 { self.status.reconnects.load(Ordering::Acquire) }
    pub async fn next(&mut self) -> Result<Option<SessionMsg>, String> {
        self.incoming.recv().await.transpose()
    }
    async fn request(&mut self, request: Request) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.commands.send(Command { request, reply }).await.map_err(|_| "the connection is closed".to_owned())?;
        receive.await.map_err(|_| "the connection is closed".to_owned())?
    }
    pub async fn subscribe(&mut self, id: SubId, query: Query) -> Result<(), String> {
        self.request(Request::Subscribe { id, query, since: None }).await
    }
    pub async fn subscribe_since(&mut self, id: SubId, query: Query, since: Version) -> Result<(), String> {
        self.request(Request::Subscribe { id, query, since: Some(since) }).await
    }
    pub async fn intent(&mut self, id: u64, intent: Intent) -> Result<(), String> {
        self.request(Request::Intent { id, intent }).await
    }
}

/// Only read_next may be cancelled. All commands and reconnection execute outside select.
#[async_trait::async_trait]
trait Wire: Send {
    fn session(&self) -> Option<SessionInfo>;
    fn reconnects(&self) -> u32;
    async fn read_next(&mut self) -> Result<Option<SessionMsg>, String>;
    async fn restore(&mut self) -> Result<(), String>;
    async fn command(&mut self, request: Request) -> Result<(), String>;
}
#[async_trait::async_trait]
impl Wire for WireClient {
    fn session(&self) -> Option<SessionInfo> { WireClient::session(self).cloned() }
    fn reconnects(&self) -> u32 { WireClient::reconnects(self) }
    async fn read_next(&mut self) -> Result<Option<SessionMsg>, String> { WireClient::read_next(self).await }
    async fn restore(&mut self) -> Result<(), String> { WireClient::restore(self).await }
    async fn command(&mut self, request: Request) -> Result<(), String> {
        match request {
            Request::Subscribe { id, query, since: Some(since) } => self.subscribe_since(id, query, since).await,
            Request::Subscribe { id, query, since: None } => self.subscribe(id, query).await,
            Request::Intent { id, intent } => self.intent(id, intent).await,
        }
    }
}

enum Ready { Message(Result<Option<SessionMsg>, String>), Command(Option<Command>), Delivered }
async fn run(mut wire: impl Wire, mut commands: mpsc::Receiver<Command>, incoming: mpsc::Sender<Result<SessionMsg, String>>, status: Arc<Status>) {
    let mut pending = None;
    let mut closing = false;
    loop {
        let ready = if pending.is_some() {
            // One held message beyond the bounded queue. Stop reading the wire, but continue
            // accepting commands: a caller awaiting intent must not deadlock behind its inbox.
            tokio::select! {
                permit = incoming.reserve() => {
                    let Ok(permit) = permit else { return };
                    permit.send(pending.take().unwrap());
                    Ready::Delivered
                }
                command = commands.recv(), if !closing => Ready::Command(command),
            }
        } else {
            tokio::select! {
                message = wire.read_next() => Ready::Message(message),
                command = commands.recv() => Ready::Command(command),
                _ = incoming.closed() => return,
            }
        };
        match ready {
            Ready::Delivered if closing => return,
            Ready::Delivered => {},
            Ready::Command(None) => return,
            Ready::Command(Some(command)) => {
                let subscription = matches!(command.request, Request::Subscribe { .. });
                let mut result = wire.command(command.request).await;
                if let Err(error) = result {
                    result = match wire.restore().await {
                        // Subscriptions are remembered by protocol::Client and restored by
                        // the handshake. An intent's uncertain write is never replayed.
                        Ok(()) if subscription => Ok(()),
                        Ok(()) => Err(error),
                        Err(reconnect) => {
                            closing = true;
                            if pending.is_none() { pending = Some(Err(reconnect.clone())); }
                            Err(format!("{error}; reconnect failed: {reconnect}"))
                        }
                    };
                }
                status.refresh(&wire);
                let _ = command.reply.send(result);
            }
            Ready::Message(Ok(Some(message))) => {
                status.refresh(&wire);
                pending = Some(Ok(message));
            }
            Ready::Message(Ok(None) | Err(_)) => {
                // This future is never selected against local input. Once it starts, handshake,
                // resubscription and partial writes run to completion or close the connection.
                if let Err(error) = wire.restore().await {
                    pending = Some(Err(error));
                    closing = true;
                }
                status.refresh(&wire);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::atomic::AtomicUsize, time::Duration};
    use tokio::sync::Semaphore;

    struct Fake {
        messages: mpsc::Receiver<Result<SessionMsg, String>>,
        events: mpsc::UnboundedSender<&'static str>,
        reads: Arc<AtomicUsize>,
        reconnects: u32,
        restore_gate: Arc<Semaphore>,
        command_gate: Arc<Semaphore>,
        fail_command: bool,
    }
    #[async_trait::async_trait]
    impl Wire for Fake {
        fn session(&self) -> Option<SessionInfo> { None }
        fn reconnects(&self) -> u32 { self.reconnects }
        async fn read_next(&mut self) -> Result<Option<SessionMsg>, String> {
            let result = self.messages.recv().await.transpose();
            self.reads.fetch_add(1, Ordering::SeqCst);
            result
        }
        async fn restore(&mut self) -> Result<(), String> {
            self.events.send("restore.started").unwrap();
            self.restore_gate.acquire().await.unwrap().forget();
            self.reconnects += 1;
            self.events.send("restore.finished").unwrap();
            Ok(())
        }
        async fn command(&mut self, _: Request) -> Result<(), String> {
            self.events.send("command.started").unwrap();
            self.command_gate.acquire().await.unwrap().forget();
            self.events.send("command.finished").unwrap();
            if std::mem::take(&mut self.fail_command) { Err("uncertain write".into()) } else { Ok(()) }
        }
    }
    struct Harness {
        client: Client,
        wire: mpsc::Sender<Result<SessionMsg, String>>,
        events: mpsc::UnboundedReceiver<&'static str>,
        reads: Arc<AtomicUsize>,
        restore: Arc<Semaphore>,
        commands: Arc<Semaphore>,
    }
    fn harness() -> Harness {
        configured(false)
    }
    fn configured(fail_command: bool) -> Harness {
        let (wire, messages) = mpsc::channel(128);
        let (events, receive_events) = mpsc::unbounded_channel();
        let reads = Arc::new(AtomicUsize::new(0));
        let restore = Arc::new(Semaphore::new(0));
        let commands = Arc::new(Semaphore::new(0));
        let client = Client::spawn(Fake { messages, events, reads: reads.clone(), reconnects: 0,
            restore_gate: restore.clone(), command_gate: commands.clone(), fail_command });
        Harness { client, wire, events: receive_events, reads, restore, commands }
    }
    async fn within<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(2), future).await.expect("actor stalled")
    }
    fn intent() -> Intent { Intent::Prompt { text: "once".into(), attachments: vec![] } }

    #[tokio::test]
    async fn cancelling_frontend_waits_does_not_interrupt_reconnection() {
        let mut test = harness();
        test.wire.send(Err("connection dropped".into())).await.unwrap();
        assert_eq!(within(test.events.recv()).await, Some("restore.started"));
        for _ in 0..12 {
            assert!(tokio::time::timeout(Duration::from_millis(1), test.client.next()).await.is_err());
        }
        assert!(test.events.try_recv().is_err(), "handshake was restarted or completed without its permit");
        test.restore.add_permits(1);
        assert_eq!(within(test.events.recv()).await, Some("restore.finished"));
        test.wire.send(Ok(SessionMsg::Ack { id: 7 })).await.unwrap();
        assert!(matches!(within(test.client.next()).await.unwrap(), Some(SessionMsg::Ack { id: 7 })));
        assert_eq!(test.client.reconnects(), 1);
    }

    #[tokio::test]
    async fn full_incoming_queue_keeps_one_pending_message_and_still_accepts_commands() {
        let mut test = harness();
        for id in 0..66 { test.wire.send(Ok(SessionMsg::Ack { id })).await.unwrap(); }
        within(async { while test.reads.load(Ordering::SeqCst) < 65 { tokio::task::yield_now().await; } }).await;
        assert_eq!(test.client.incoming.len(), CAPACITY);
        assert_eq!(test.reads.load(Ordering::SeqCst), CAPACITY + 1);
        test.commands.add_permits(1);
        within(test.client.intent(100, intent())).await.unwrap();
        assert_eq!(within(test.events.recv()).await, Some("command.started"));
        assert_eq!(within(test.events.recv()).await, Some("command.finished"));
        assert_eq!(test.reads.load(Ordering::SeqCst), CAPACITY + 1, "wire reads must stop while the inbox is full");
        for expected in 0..66 {
            assert!(matches!(within(test.client.next()).await.unwrap(), Some(SessionMsg::Ack { id }) if id == expected));
        }
    }

    #[tokio::test]
    async fn cancelling_a_request_wait_never_cancels_or_repeats_its_write() {
        let mut test = harness();
        assert!(tokio::time::timeout(Duration::from_millis(5), test.client.intent(1, intent())).await.is_err());
        assert_eq!(within(test.events.recv()).await, Some("command.started"));
        for _ in 0..5 { assert!(tokio::time::timeout(Duration::from_millis(1), test.client.next()).await.is_err()); }
        test.commands.add_permits(1);
        assert_eq!(within(test.events.recv()).await, Some("command.finished"));
        test.wire.send(Ok(SessionMsg::Ack { id: 1 })).await.unwrap();
        assert!(matches!(within(test.client.next()).await.unwrap(), Some(SessionMsg::Ack { id: 1 })));
        assert!(test.events.try_recv().is_err(), "the cancelled caller's intent was replayed");
    }
    #[tokio::test]
    async fn an_uncertain_intent_write_restores_the_connection_without_replaying_the_intent() {
        let mut test = configured(true);
        test.commands.add_permits(1);
        test.restore.add_permits(1);
        assert_eq!(within(test.client.intent(1, intent())).await.unwrap_err(), "uncertain write");
        for expected in ["command.started", "command.finished", "restore.started", "restore.finished"] {
            assert_eq!(within(test.events.recv()).await, Some(expected));
        }
        assert_eq!(test.client.reconnects(), 1);
        test.wire.send(Ok(SessionMsg::Ack { id: 2 })).await.unwrap();
        assert!(matches!(within(test.client.next()).await.unwrap(), Some(SessionMsg::Ack { id: 2 })));
        assert!(test.events.try_recv().is_err());
    }
}
