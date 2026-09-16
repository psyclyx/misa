//! Bounded logical messages; chunks are framing, never partial publications.
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;

pub struct Frame<T> {
    pub message: T,
    /// Retain through delivery queues; releasing accounts for consumed wire data.
    pub permit: Option<OwnedSemaphorePermit>,
}

pub struct Reader {
    recv: RecvStream,
    header: [u8; 4],
    header_len: usize,
    remaining: usize,
    more: bool,
    body: Vec<u8>,
    limit: usize,
    prefix: [u8; 8],
    prefix_len: usize,
    wire_remaining: usize,
    wire_size: usize,
    budget: Option<Arc<Semaphore>>,
    permit: Option<OwnedSemaphorePermit>,
}
impl Reader {
    pub fn new(recv: RecvStream, limit: usize) -> Self {
        Self {
            recv,
            header: [0; 4],
            header_len: 0,
            remaining: 0,
            more: false,
            body: Vec::new(),
            limit,
            prefix: [0; 8],
            prefix_len: 0,
            wire_remaining: 0,
            wire_size: 0,
            budget: None,
            permit: None,
        }
    }
    pub fn with_budget(recv: RecvStream, limit: usize, budget: Arc<Semaphore>) -> Self {
        let mut reader = Self::new(recv, limit);
        reader.budget = Some(budget);
        reader
    }
    /// Safe to cancel while awaiting bytes; all partial framing state is owned.
    pub async fn next<T: DeserializeOwned>(&mut self) -> Result<Option<T>, String> {
        Ok(self.next_budgeted().await?.map(|frame| frame.message))
    }
    pub async fn next_budgeted<T: DeserializeOwned>(&mut self) -> Result<Option<Frame<T>>, String> {
        loop {
            while self.prefix_len < 8 {
                let Some(read) = self
                    .recv
                    .read(&mut self.prefix[self.prefix_len..])
                    .await
                    .map_err(|error| error.to_string())?
                else {
                    return if self.prefix_len == 0 {
                        Ok(None)
                    } else {
                        Err("Truncated scoped frame prefix".into())
                    };
                };
                self.prefix_len += read;
                if self.prefix_len == 8 {
                    self.wire_size = usize::try_from(u64::from_be_bytes(self.prefix))
                        .map_err(|_| "Frame length overflow")?;
                    let maximum = self.limit.saturating_add(
                        self.limit
                            .div_ceil(misa_proto::chunk::MAX_CHUNK)
                            .max(1)
                            .saturating_mul(4),
                    );
                    if self.wire_size < 4
                        || self.wire_size > maximum
                        || self.wire_size > u32::MAX as usize - 8
                    {
                        return Err("Scoped message exceeds byte limit".into());
                    }
                    self.wire_remaining = self.wire_size;
                }
            }
            if let Some(budget) = &self.budget {
                if self.permit.is_none() {
                    self.permit = Some(
                        budget
                            .clone()
                            .acquire_many_owned((self.wire_size + 8) as u32)
                            .await
                            .map_err(|_| "Receive byte budget closed")?,
                    );
                }
            }
            if self.header_len < 4 {
                if self.wire_remaining < 4 - self.header_len {
                    return Err("Truncated scoped chunk header".into());
                }
                let read = self
                    .recv
                    .read(&mut self.header[self.header_len..])
                    .await
                    .map_err(|error| error.to_string())?;
                let Some(read) = read else {
                    return Err("Truncated scoped message".into());
                };
                self.header_len += read;
                self.wire_remaining -= read;
                if self.header_len < 4 {
                    continue;
                }
                let word = u32::from_be_bytes(self.header);
                self.more = word & (1 << 31) != 0;
                self.remaining = (word & !(1 << 31)) as usize;
                if self.remaining > misa_proto::chunk::MAX_CHUNK
                    || self.remaining > self.limit.saturating_sub(self.body.len())
                    || self.remaining > self.wire_remaining
                {
                    return Err("Scoped message exceeds byte limit".into());
                }
                if self.remaining == 0 && self.more {
                    return Err("Empty continuation chunk".into());
                }
            }
            if self.remaining != 0 {
                let mut bytes = [0; 16 * 1024];
                let count = bytes.len().min(self.remaining);
                let Some(read) = self
                    .recv
                    .read(&mut bytes[..count])
                    .await
                    .map_err(|error| error.to_string())?
                else {
                    return Err("Truncated scoped message".into());
                };
                self.body.extend_from_slice(&bytes[..read]);
                self.remaining -= read;
                self.wire_remaining -= read;
                if self.remaining != 0 {
                    continue;
                }
            }
            self.header_len = 0;
            if !self.more {
                if self.wire_remaining != 0 {
                    return Err("Trailing bytes after scoped logical message".into());
                }
                let body = std::mem::take(&mut self.body);
                let message =
                    misa_proto::chunk::decode(&body).map_err(|error| error.to_string())?;
                self.prefix_len = 0;
                return Ok(Some(Frame {
                    message,
                    permit: self.permit.take(),
                }));
            }
        }
    }
}

pub struct Writer {
    send: SendStream,
    limit: usize,
    budget: Option<Arc<Semaphore>>,
}
impl Writer {
    pub fn new(send: SendStream, limit: usize) -> Self {
        Self {
            send,
            limit,
            budget: None,
        }
    }
    pub fn with_budget(mut self, budget: Arc<Semaphore>) -> Self {
        self.budget = Some(budget);
        self
    }
    pub fn finish(&mut self) -> Result<(), String> {
        self.send.finish().map_err(|error| error.to_string())
    }
    /// Run in a dedicated IO owner. Cancelling this future can truncate a frame.
    pub async fn send<T: Serialize>(&mut self, message: &T) -> Result<(), String> {
        let size = misa_proto::chunk::encoded_size_with_limit(message, self.limit)
            .map_err(|error| error.to_string())?;
        let _permit = match &self.budget {
            Some(budget) => Some(
                budget
                    .clone()
                    .acquire_many_owned(
                        u32::try_from(size + 8).map_err(|_| "Frame length overflow")?,
                    )
                    .await
                    .map_err(|_| "Send byte budget closed")?,
            ),
            None => None,
        };
        let bytes = misa_proto::chunk::encode_with_limit(message, self.limit)
            .map_err(|error| error.to_string())?;
        if bytes.len() != size {
            return Err("Serialization changed during byte reservation".into());
        }
        self.send
            .write_all(&(size as u64).to_be_bytes())
            .await
            .map_err(|error| error.to_string())?;
        self.send
            .write_all(&bytes)
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::protocol::{AcceptError, ProtocolHandler, Router};
    use std::{sync::Arc, time::Duration};
    use tokio::sync::Semaphore;
    #[derive(Debug)]
    struct Fragmented {
        bytes: Vec<u8>,
        gate: Arc<Semaphore>,
        started: Arc<Semaphore>,
    }
    impl ProtocolHandler for Fragmented {
        async fn accept(&self, connection: iroh::endpoint::Connection) -> Result<(), AcceptError> {
            let (mut send, mut recv) = connection.accept_bi().await?;
            let mut byte = [0];
            let _ = recv.read(&mut byte).await;
            send.write_all(&self.bytes[..2])
                .await
                .map_err(std::io::Error::other)?;
            self.started.add_permits(1);
            self.gate.acquire().await.unwrap().forget();
            send.write_all(&self.bytes[2..])
                .await
                .map_err(std::io::Error::other)?;
            let _ = send.finish();
            connection.closed().await;
            Ok(())
        }
    }
    #[tokio::test]
    async fn cancelled_read_retains_partial_header_and_rejects_logical_oversize() {
        for limit in [1024, 2] {
            let server = crate::iroh::bind(None, false).await.unwrap();
            let endpoint = crate::iroh::bind(None, false).await.unwrap();
            let gate = Arc::new(Semaphore::new(0));
            let started = Arc::new(Semaphore::new(0));
            let message = "a complete bounded message".to_string();
            let bytes = misa_proto::chunk::encode(&message).unwrap();
            let mut framed = (bytes.len() as u64).to_be_bytes().to_vec();
            framed.extend(bytes);
            let handler = Fragmented {
                bytes: framed,
                gate: gate.clone(),
                started: started.clone(),
            };
            let router = Router::builder(server.clone())
                .accept(b"/misa/test-reader", handler)
                .spawn();
            let address = crate::iroh::address_of(&crate::iroh::node_of(&server)).unwrap();
            let connection = endpoint
                .connect(address, b"/misa/test-reader")
                .await
                .unwrap();
            let (mut send, recv) = connection.open_bi().await.unwrap();
            send.write_all(b"x").await.unwrap();
            started.acquire().await.unwrap().forget();
            let mut reader = Reader::new(recv, limit);
            assert!(
                tokio::time::timeout(Duration::from_millis(20), reader.next::<String>())
                    .await
                    .is_err()
            );
            assert_eq!(reader.prefix_len, 2);
            gate.add_permits(1);
            let received = tokio::time::timeout(Duration::from_secs(2), reader.next::<String>())
                .await
                .unwrap();
            if limit == 1024 {
                assert_eq!(received.unwrap(), Some(message));
            } else {
                assert!(received.is_err());
                assert!(reader.body.is_empty());
            }
            connection.close(0u32.into(), b"done");
            router.shutdown().await.unwrap();
            endpoint.close().await;
            server.close().await;
        }
    }
    #[test]
    fn encoding_limit_is_logical_not_only_a_chunk_limit() {
        assert!(misa_proto::chunk::encode_with_limit(&"x".repeat(1024), 32).is_err());
        assert!(misa_proto::chunk::encode_with_limit(&"x".repeat(1024), 2048).is_ok());
    }
}
