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
                    let result = async {
                        let blob = tokio::time::timeout(TIMEOUT, blobs.get(&reference.hash)).await
                            .map_err(|_| "The attachment download timed out".to_string())??.ok_or("This attachment is no longer available")?;
                        if blob.hash != reference.hash || blob.bytes.len() as u64 != reference.len { return Err("The attachment bytes do not match the offered file".to_string()); }
                        let path = destination.clone();
                        tokio::task::spawn_blocking(move || crate::save::write_new(&path, &blob.bytes)).await.map_err(|error| error.to_string())?
                    }.await;
                    SessionReply::Notice(match result { Ok(()) => format!("Saved {destination}"), Err(error) => error })
                });
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Session, Presentation};
    struct NoBlobs;
    impl misa_transport::blob::BlobStore for NoBlobs {
        fn get(&self, _: &str) -> Option<Vec<u8>> { None }
        fn media(&self, _: &str) -> Option<String> { None }
        fn has(&self, _: &str) -> bool { false }
        fn store(&self, _: Vec<u8>, _: Option<&str>) -> Result<misa_proto::view::BlobRef, String> { Err("unused".into()) }
    }
    async fn reply(client: &mut Remote) -> SessionReply {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop { if let Some(Presentation::Reply(reply)) = client.next_presentation().await.unwrap() { return reply; } }
        }).await.expect("reply stalled")
    }
    #[tokio::test]
    async fn rejected_drafts_and_late_correlated_faults_leave_the_connection_usable() {
        let kernel = std::sync::Arc::new(misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::always("done")));
        let runtime = misa_session::Runtime::start("requests", "Requests", None, kernel, "scripted", "test", misa_value::Value::Null);
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let sessions = misa_transport::iroh::Sessions::new(); sessions.insert(runtime);
        let router = misa_transport::server::serve(endpoint.clone(), sessions, std::sync::Arc::new(NoBlobs), std::sync::Arc::new(misa_transport::admission::Admission::open()));
        let ticket = misa_transport::iroh::ticket(&endpoint, "requests").to_string();
        let mut client = Remote::attach(&ticket).await.unwrap();
        assert!(client.request(SessionRequest::Intent(Intent::Interrupt { text: " ".into(), attachments: vec![] })).await.is_none());
        assert!(matches!(reply(&mut client).await, SessionReply::Sent { draft: Some((text, refs)), result: Err(_) } if text == " " && refs.is_empty()));
        assert!(client.request(SessionRequest::Intent(Intent::Action { node: "missing".into(), action: "missing".into(), args: misa_value::Value::Null, fields: vec![] })).await.is_none());
        // Force expiration before reading the already-in-flight correlated fault.
        assert!(matches!(client.requests.expire(), SessionReply::Sent { result: Err(_), .. }));
        assert!(matches!(reply(&mut client).await, SessionReply::Notice(_)));
        assert!(client.request(SessionRequest::Intent(Intent::Prompt { text: "still connected".into(), attachments: vec![] })).await.is_none());
        assert!(matches!(reply(&mut client).await, SessionReply::Sent { draft: Some((text, _)), result: Ok(()) } if text == "still connected"));
        router.shutdown().await.unwrap(); endpoint.close().await;
    }
}
