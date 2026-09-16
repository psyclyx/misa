//! Socket/framing owner only. Replicas and directed replies belong to misa-client.
use super::scoped_io::{Frame, MAX_MESSAGE, Reader, Writer};
use iroh::{Endpoint, EndpointAddr};
use misa_proto::{
    ClientInfo,
    observation::Scope,
    scoped::{self, ClientMessage, Lane, ServerMessage},
};
use std::sync::Arc;
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};

pub enum Event {
    Message(Frame<ServerMessage>),
    LaneClosed { lane: Lane, reason: String },
}

#[derive(Clone, Debug)]
pub struct Welcome {
    pub daemon: String,
    pub scope: Scope,
}
pub struct Client {
    pub welcome: Welcome,
    outgoing: mpsc::Sender<ClientMessage>,
    incoming: mpsc::Receiver<Result<Event, String>>,
    data: mpsc::Receiver<Result<Event, String>>,
    reader: tokio::task::AbortHandle,
    writer: tokio::task::AbortHandle,
    lanes: tokio::task::AbortHandle,
    connection: iroh::endpoint::Connection,
}
impl Drop for Client {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
        self.lanes.abort();
        self.connection.close(0u32.into(), b"client closed");
    }
}
impl Client {
    pub async fn connect(
        endpoint: &Endpoint,
        address: EndpointAddr,
        info: ClientInfo,
    ) -> Result<Self, String> {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Self::handshake(endpoint, address, info),
        )
        .await
        .map_err(|_| "Scoped handshake timed out".to_string())?
    }
    async fn handshake(
        endpoint: &Endpoint,
        address: EndpointAddr,
        info: ClientInfo,
    ) -> Result<Self, String> {
        let connection = endpoint
            .connect(address, scoped::ALPN)
            .await
            .map_err(|error| error.to_string())?;
        // Persistent observation lanes consume stream credit for their lifetime.
        // Match the bounded receiver task cap, leaving room for finite reads.
        connection.set_max_concurrent_uni_streams(160u32.into());
        let (send, recv) = connection
            .open_bi()
            .await
            .map_err(|error| error.to_string())?;
        let control_budget = Arc::new(Semaphore::new(8 * 1024 * 1024));
        let mut writer = Writer::new(send, 4 * 1024 * 1024).with_budget(control_budget.clone());
        let mut reader = Reader::with_budget(recv, 4 * 1024 * 1024, control_budget);
        writer
            .send(&ClientMessage::Hello {
                version: scoped::VERSION,
                client: info,
            })
            .await?;
        let message: ServerMessage = reader
            .next()
            .await?
            .ok_or("Daemon closed before greeting")?;
        message
            .validate_welcome(&connection.remote_id().to_string())
            .map_err(|fault| fault.message)?;
        let welcome = match message {
            ServerMessage::Welcome { daemon, scope, .. }
                if daemon == connection.remote_id().to_string() =>
            {
                Welcome { daemon, scope }
            }
            ServerMessage::Fault { fault } => return Err(fault.message),
            _ => return Err("Daemon greeting does not match authenticated endpoint".into()),
        };
        let (outgoing, mut commands) = mpsc::channel::<ClientMessage>(64);
        let (messages, incoming) = mpsc::channel(8);
        let (data_messages, data) = mpsc::channel(8);
        let faults = messages.clone();
        let write_task = tokio::spawn(async move {
            while let Some(command) = commands.recv().await {
                let result =
                    tokio::time::timeout(std::time::Duration::from_secs(20), writer.send(&command))
                        .await
                        .map_err(|_| "Scoped write timed out".to_string())
                        .and_then(|result| result);
                if let Err(error) = result {
                    let _ = faults.send(Err(error)).await;
                    return;
                }
            }
        })
        .abort_handle();
        let lane_faults = messages.clone();
        let read_task = tokio::spawn(async move {
            loop {
                let message = match reader.next_budgeted::<ServerMessage>().await {
                    Ok(Some(frame)) => match &frame.message {
                        ServerMessage::Reply { .. }
                        | ServerMessage::Fault { .. }
                        | ServerMessage::ReadReply { .. } => Ok(Event::Message(frame)),
                        ServerMessage::Publication {
                            publication: misa_proto::observation::Publication::Closed { .. },
                        } => Ok(Event::Message(frame)),
                        _ => Err("Unexpected payload on scoped control lane".into()),
                    },
                    Ok(None) => Err("Daemon disconnected".into()),
                    Err(error) => Err(error),
                };
                let failed = message.is_err();
                if messages.send(message).await.is_err() || failed {
                    return;
                }
            }
        })
        .abort_handle();
        let lanes_connection = connection.clone();
        let lanes = tokio::spawn(async move {
            let budget = Arc::new(Semaphore::new(128 * 1024 * 1024));
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    _ = tasks.join_next(), if !tasks.is_empty() => {},
                    incoming = lanes_connection.accept_uni(), if tasks.len() < 160 => {
                        let recv = match incoming { Ok(recv) => recv, Err(error) => { let _ = lane_faults.send(Err(error.to_string())).await; return; } };
                        let messages = data_messages.clone(); let budget = budget.clone();
                        tasks.spawn(async move {
                            let mut reader = Reader::with_budget(recv, MAX_MESSAGE, budget);
                            let lane = match tokio::time::timeout(std::time::Duration::from_secs(5), reader.next::<Lane>()).await {
                                Ok(Ok(Some(lane))) => lane,
                                _ => { let _ = messages.send(Err("Invalid scoped data lane header".into())).await; return; }
                            };
                            loop {
                                let frame = match reader.next_budgeted::<ServerMessage>().await {
                                    Ok(Some(frame)) => frame,
                                    result => { let reason = result.err().unwrap_or_else(|| "Data lane ended before completion".into()); let _ = messages.send(Ok(Event::LaneClosed { lane, reason })).await; return; }
                                };
                                let valid = match (&lane, &frame.message) {
                                    (Lane::Observation { handle }, ServerMessage::Publication { publication }) => publication.handle() == *handle,
                                    (Lane::Read { id }, ServerMessage::ReadReply { reply }) => reply.id == *id,
                                    _ => false,
                                };
                                if !valid { let _ = messages.send(Ok(Event::LaneClosed { lane, reason: "Payload does not match data lane identity".into() })).await; return; }
                                let done = matches!(lane, Lane::Read { .. }) || matches!(&frame.message, ServerMessage::Publication { publication: misa_proto::observation::Publication::Closed { .. } });
                                if messages.send(Ok(Event::Message(frame))).await.is_err() || done { return; }
                            }
                        });
                    }
                }
            }
        }).abort_handle();
        Ok(Self {
            welcome,
            outgoing,
            incoming,
            data,
            reader: read_task,
            writer: write_task,
            lanes,
            connection,
        })
    }
    /// Queue acceptance is distinct from remote execution. A refused enqueue has
    /// not been written; a later transport failure leaves its outcome uncertain.
    pub fn try_send(&self, message: ClientMessage) -> Result<(), String> {
        message.validate().map_err(|fault| fault.message)?;
        self.outgoing
            .try_send(message)
            .map_err(|error| error.to_string())
    }
    /// Wait for one control-queue slot without borrowing the receive owner.
    /// Dropping this future or its permit is cancellation safe: no frame was sent.
    pub fn reserve(
        &self,
    ) -> impl std::future::Future<Output = Result<mpsc::OwnedPermit<ClientMessage>, String>>
    + Send
    + 'static
    + use<> {
        let sender = self.outgoing.clone();
        async move {
            sender
                .reserve_owned()
                .await
                .map_err(|_| "Scoped write owner stopped".to_string())
        }
    }
    pub async fn next(&mut self) -> Result<Event, String> {
        tokio::select! {
            biased;
            value = self.incoming.recv() => value.ok_or("Scoped control owner stopped")?,
            value = self.data.recv() => value.ok_or("Scoped data owner stopped")?,
        }
    }
}
