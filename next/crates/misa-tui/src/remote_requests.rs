//! Directed replies are correlated independently of view delivery. Both pending
//! replies and downloads share one bound; no transcript inbox is needed.
use std::collections::HashMap;
use std::time::Duration;
use crate::{Remote, SessionRequest, SessionReply};
use misa_proto::{SessionMsg, Intent};
const LIMIT: usize = 16;
const TIMEOUT: Duration = Duration::from_secs(20);
#[derive(Default)]
pub(super) struct Pending {
    entries: HashMap<u64, (tokio::time::Instant, SessionRequest)>,
    pub transfers: tokio::task::JoinSet<SessionReply>,
}
impl Pending {
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    pub fn deadline(&self) -> tokio::time::Instant {
        self.entries.values().map(|(deadline, _)| *deadline).min().unwrap_or_else(|| tokio::time::Instant::now() + TIMEOUT)
    }
    pub fn expire(&mut self) -> SessionReply {
        let id = *self.entries.iter().min_by_key(|(_, (deadline, _))| *deadline).unwrap().0;
        let (_, request) = self.entries.remove(&id).unwrap();
        failed(request, "The session did not answer the request".into())
    }
}
fn failed(request: SessionRequest, error: String) -> SessionReply {
    match request {
        SessionRequest::Intent(intent) => SessionReply::Sent { draft: draft(&intent), result: Err(error) },
        SessionRequest::Upload { generation, .. } => SessionReply::Uploaded { generation, result: Err(error) },
        SessionRequest::Complete { source, prefix } => SessionReply::Complete { source, prefix, result: Err(error) },
        _ => SessionReply::Notice(error),
    }
}
fn draft(intent: &Intent) -> Option<(String, Vec<misa_proto::view::BlobRef>)> {
    match intent { Intent::Prompt { text, attachments } | Intent::Interrupt { text, attachments } => Some((text.clone(), attachments.clone())), _ => None }
}
impl Remote {
    pub(super) async fn submit_request(&mut self, request: SessionRequest) -> Option<SessionReply> {
        let limit = if matches!(request, SessionRequest::Intent(_)) { LIMIT * 2 } else { LIMIT };
        if self.requests.entries.len() + self.requests.transfers.len() >= limit {
            return Some(failed(request, "Too many pending requests; try again shortly".into()));
        }
        if let SessionRequest::Upload { generation, bytes, media } = request {
            let blobs = self.blobs.clone();
            self.requests.transfers.spawn(async move {
                let result = tokio::time::timeout(TIMEOUT, blobs.share(bytes, Some(&media))).await
                    .map_err(|_| "The attachment upload timed out".to_string()).and_then(|result| result);
                SessionReply::Uploaded { generation, result }
            });
            return None;
        }
        let id = crate::next_intent_id();
        let intent = match &request {
            SessionRequest::Complete { source, prefix } => Intent::Complete { source: source.clone(), prefix: prefix.clone(), limit: None },
            SessionRequest::Save { node, .. } => Intent::Action { node: node.clone(), action: "attachment.save".into(), args: misa_value::Value::Null, fields: vec![] },
            SessionRequest::Intent(intent) => intent.clone(),
            SessionRequest::Upload { .. } => unreachable!(),
        };
        match self.client.intent(id, intent).await {
            Ok(()) => { self.requests.entries.insert(id, (tokio::time::Instant::now() + TIMEOUT, request)); None }
            Err(error) => Some(failed(request, error)),
        }
    }
    pub(super) fn request_reply(&mut self, message: &SessionMsg) -> Option<SessionReply> {
        match message {
            SessionMsg::Ack { id } if self.requests.entries.get(id).is_some_and(|(_, request)| matches!(request, SessionRequest::Intent(_))) => {
                let (_, SessionRequest::Intent(intent)) = self.requests.entries.remove(id).unwrap() else { unreachable!() };
                Some(SessionReply::Sent { draft: draft(&intent), result: Ok(()) })
            }
            SessionMsg::Completion { id, candidates, truncated, .. } => {
                let (_, request) = self.requests.entries.remove(id)?;
                let SessionRequest::Complete { source, prefix } = request else { return Some(failed(request, "Unexpected completion reply".into())); };
                Some(SessionReply::Complete { source, prefix, result: Ok((candidates.clone(), *truncated)) })
            }
            SessionMsg::Fault { id: Some(id), fault } => {
                let (_, request) = self.requests.entries.remove(id)?;
                Some(failed(request, fault.message.clone()))
            }
            SessionMsg::Download { id, download } => {
                let (_, request) = self.requests.entries.remove(id)?;
                let SessionRequest::Save { destination, .. } = request else { return Some(failed(request, "Unexpected download reply".into())); };
                let reference = match download.blob.clone() { Some(reference) => reference, None => return Some(SessionReply::Notice(download.error.clone())) };
                let blobs = self.blobs.clone();
                self.requests.transfers.spawn(async move {
                    let result = tokio::time::timeout(TIMEOUT, async {
                        let blob = blobs.get(&reference.hash).await?.ok_or("This attachment is no longer available")?;
                        if blob.hash != reference.hash || blob.bytes.len() as u64 != reference.len { return Err("The attachment bytes do not match the offered file".to_string()); }
                        let path = destination.clone();
                        tokio::task::spawn_blocking(move || crate::save::write_new(&path, &blob.bytes)).await.map_err(|error| error.to_string())?
                    }).await.map_err(|_| "The attachment download timed out".to_string()).and_then(|result| result);
                    SessionReply::Notice(match result { Ok(()) => format!("Saved {destination}"), Err(error) => error })
                });
                None
            }
            _ => None,
        }
    }
}
