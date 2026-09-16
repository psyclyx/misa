//! Admission only: pairing grants the authenticated endpoint key access to a daemon.
//! It never selects a session or opens an application subscription.
use crate::{
    admission::Admission,
    scoped_io::{Reader, Writer},
};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

pub const ALPN: &[u8] = b"/misa/pair/1";
const LIMIT: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Serialize, Deserialize)]
struct Request {
    code: String,
    label: String,
}
#[derive(Serialize, Deserialize)]
struct Reply {
    accepted: bool,
    message: String,
}

pub struct Handler {
    pub admission: Arc<Admission>,
}
impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("pairing handler")
    }
}
impl ProtocolHandler for Handler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let _ = tokio::time::timeout(TIMEOUT, async {
            if !self.admission.is_inviting(now_ms()) {
                return Ok::<_, String>(());
            }
            let (send, recv) = connection.accept_bi().await.map_err(|e| e.to_string())?;
            let Some(request) = Reader::new(recv, LIMIT).next::<Request>().await? else {
                return Ok(());
            };
            let result = if request.code.len() > 128 || request.label.len() > 256 {
                Err("pairing fields exceed their limits".to_owned())
            } else {
                self.admission.accept(
                    &connection.remote_id().to_string(),
                    &request.code,
                    &request.label,
                    now_ms(),
                )
            };
            let reply = match result {
                Ok(()) => Reply {
                    accepted: true,
                    message: "paired".into(),
                },
                Err(message) => Reply {
                    accepted: false,
                    message,
                },
            };
            let mut writer = Writer::new(send, LIMIT);
            writer.send(&reply).await?;
            writer.finish()?;
            // Keep QUIC alive until the client has received the response and closes.
            connection.closed().await;
            Ok(())
        })
        .await;
        Ok(())
    }
}

pub async fn pair(
    endpoint: &Endpoint,
    address: EndpointAddr,
    code: &str,
    label: &str,
) -> Result<String, String> {
    if code.len() > 128 || label.len() > 256 {
        return Err("pairing fields exceed their limits".into());
    }
    tokio::time::timeout(TIMEOUT, async {
        let connection = endpoint
            .connect(address, ALPN)
            .await
            .map_err(|e| e.to_string())?;
        let (send, recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
        let mut writer = Writer::new(send, LIMIT);
        writer
            .send(&Request {
                code: code.into(),
                label: label.into(),
            })
            .await?;
        writer.finish()?;
        let reply = Reader::new(recv, LIMIT)
            .next::<Reply>()
            .await?
            .ok_or("daemon closed pairing without a reply")?;
        connection.close(0u32.into(), b"pairing finished");
        if reply.accepted {
            Ok(reply.message)
        } else {
            Err(reply.message)
        }
    })
    .await
    .map_err(|_| "pairing timed out".to_owned())?
}
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pairing_grants_only_authenticated_key_and_consumes_code() {
        let server = crate::iroh::bind(None, false).await.unwrap();
        let client = crate::iroh::bind(None, false).await.unwrap();
        let stranger = crate::iroh::bind(None, false).await.unwrap();
        let admission = Arc::new(Admission::listed([]));
        let invitation = admission.invite(60_000, now_ms());
        let address = crate::iroh::address_of(&crate::iroh::node_of(&server)).unwrap();
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(
                ALPN,
                Handler {
                    admission: admission.clone(),
                },
            )
            .spawn();
        assert!(
            pair(&client, address.clone(), "wrong", "client")
                .await
                .is_err()
        );
        assert!(!admission.admits(&client.id().to_string()));
        pair(&client, address.clone(), invitation.code(), "client")
            .await
            .unwrap();
        assert!(admission.admits(&client.id().to_string()));
        assert!(!admission.admits(&stranger.id().to_string()));
        assert!(
            pair(&stranger, address, invitation.code(), "stranger")
                .await
                .is_err()
        );
        assert!(admission.revoke(&client.id().to_string()).unwrap());
        assert!(!admission.admits(&client.id().to_string()));
        router.shutdown().await.unwrap();
        client.close().await;
        stranger.close().await;
    }
}
