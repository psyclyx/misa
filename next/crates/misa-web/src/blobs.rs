//! Blob HTTP delivery and local attachment staging, separate from document replicas.
use std::sync::Arc;
use axum::{extract::{State, Form, Multipart}, response::{Response, IntoResponse, Html, Redirect}, http::{StatusCode, header}};
use misa_proto::view::BlobRef;
use misa_value::Value;
use crate::{Remote, escape, remote};
#[cfg(test)]
use crate::{Runtime, next_id};

/// The daemon's blob connection; tests may inject an in-memory store.
pub enum Source {
    /// An in-memory test adapter.
    #[cfg(test)]
    Local(Arc<misa_kernel::Blobs>),
    /// The bytes are on the daemon that hosts the session.
    Remote(Arc<misa_client::transfers::Transfers>),
}

impl Source {
    /// Put bytes here, and get back the name they now have.
    ///
    /// The one write this client makes. It is content addressing on both sides — the name is
    /// the hash of the bytes — so an upload that has been made before costs a question and no
    /// bytes, which is what makes attaching the same screenshot twice free.
    pub async fn put(&self, bytes: Vec<u8>, media: Option<&str>) -> Result<BlobRef, String> {
        match self {
            #[cfg(test)]
            Source::Local(blobs) => blobs.put(&bytes, media),
            Source::Remote(store) => store.share(bytes, media).await,
        }
    }

    /// The bytes a hash names, and what they are.
    pub async fn get(&self, hash: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        let unknown = || "application/octet-stream".to_string();
        match self {
            #[cfg(test)]
            Source::Local(blobs) => {
                Ok(blobs.get(hash).map(|bytes| (blobs.media(hash).unwrap_or_else(unknown), bytes)))
            }
            Source::Remote(store) => {
                Ok(store.get(hash).await?.map(|blob| (blob.media.unwrap_or_else(unknown), blob.bytes)))
            }
        }
    }
}

/// One blob, for an `<img>` a view asked for.
///
/// Immutable, because a blob is named by the hash of its own content: the bytes behind a name
/// can never change, so a browser may keep them as long as it likes and scrolling back through
/// a transcript fetches nothing. That is what makes serving every request straight from the
/// store — with no cache here, and no invalidation to get wrong — the right shape.
pub(crate) async fn blob_response(source: Option<&Source>, hash: &str) -> Response {
    let Some(source) = source else {
        return (StatusCode::NOT_FOUND, "this client has no blob store").into_response();
    };
    if !misa_proto::blob::valid_hash(hash) {
        // A name that is not a content hash names nothing, and answering before asking is what
        // keeps a path-shaped name from being a path.
        return (StatusCode::NOT_FOUND, "not a content hash").into_response();
    }
    match source.get(hash).await {
        Ok(Some((media, bytes))) => (
            [
                (header::CONTENT_TYPE, media),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable".to_string()),
            ],
            bytes,
        )
            .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no such blob").into_response(),
        Err(message) => (StatusCode::BAD_GATEWAY, message).into_response(),
    }
}

async fn download_response(source: Option<&Source>, blob: BlobRef) -> Response {
    let mut response = blob_response(source, &blob.hash).await;
    if response.status() == StatusCode::OK {
        let name = attachment_name(&blob);
        response.headers_mut().insert(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\"").parse().unwrap(),
        );
    }
    response
}

#[cfg(test)]
pub(crate) async fn local_download(
    runtime: &Runtime,
    source: &Source,
    fields: std::collections::HashMap<String, String>,
) -> Response {
    use misa_protocol::invocation::{CallContext, Dispatcher};
    let dispatcher = Dispatcher::new(CallContext { principal: "web-test".into(), connection: next_id() }, 1, misa_proto::schema::Limits::default(), misa_proto::schema::Limits::default());
    let reply = dispatcher.dispatch(runtime, misa_proto::invocation::Invocation {
        id: next_id(), scope: runtime.scope(), command: "session.attachment.resolve".into(),
        input: Value::map([("node", Value::str(fields.get("node").map(String::as_str).unwrap_or("")))]),
    }).await;
    match reply.outcome {
        misa_proto::invocation::Outcome::Completed { value } => match misa_client::interface::decode::<BlobRef>(&value) {
            Ok(blob) => download_response(Some(source), blob).await,
            Err(fault) => (StatusCode::BAD_GATEWAY, fault.message).into_response(),
        },
        misa_proto::invocation::Outcome::Rejected { fault } => (StatusCode::BAD_REQUEST, fault.message).into_response(),
        _ => StatusCode::BAD_GATEWAY.into_response(),
    }
}

fn attachment_name(blob: &BlobRef) -> String {
    let extension = match blob.media.as_deref() {
        Some("image/png") => "png", Some("image/jpeg") => "jpg", Some("image/webp") => "webp",
        Some("image/gif") => "gif", Some("application/pdf") => "pdf", Some("text/plain") => "txt", _ => "bin",
    };
    format!("{}.{extension}", blob.hash)
}

pub(crate) async fn remote_download(
    State(state): State<Arc<Remote>>,
    Form(fields): Form<std::collections::HashMap<String, String>>,
) -> Response {
    let input = Value::map([("node", Value::str(fields.get("node").map(String::as_str).unwrap_or("")))]);
    let result = async {
        let prepared = state.interaction.invoke("session.attachment.resolve", input).map_err(|fault| fault.message)?;
        let value = match remote::invoke(&state, prepared).await? {
            misa_proto::invocation::Outcome::Completed { value } => value,
            misa_proto::invocation::Outcome::Rejected { fault }
            | misa_proto::invocation::Outcome::Indeterminate { fault } => return Err(fault.message),
            misa_proto::invocation::Outcome::Accepted { .. } => return Err("Attachment resolution did not complete".into()),
        };
        misa_client::interface::decode::<BlobRef>(&value).map_err(|fault| fault.message)
    }.await;
    match result {
        Ok(blob) => download_response(state.blobs.as_deref(), blob).await,
        Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
    }
}

pub(crate) async fn upload(
    source: Option<&Source>,
    pending: &std::sync::Mutex<Vec<BlobRef>>,
    mut multipart: Multipart,
) -> Response {
    let Some(source) = source else {
        return upload_fault("this client has nowhere to put a file");
    };
    let field = match multipart.next_field().await {
        Ok(Some(field)) => field,
        Ok(None) => return upload_fault("there was nothing to attach"),
        Err(error) => return upload_fault(&format!("the upload could not be read: {error}")),
    };
    let media = field.content_type().map(str::to_string);
    let bytes = match field.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => return upload_fault(&format!("the upload could not be read: {error}")),
    };
    // Refused here as well as there, because a daemon should not have to receive a gigabyte to
    // say no to it.
    if bytes.len() > misa_proto::blob::MAX_BLOB_BYTES {
        return upload_fault(&format!(
            "{} bytes is larger than the {MAX_BLOB_BYTES} byte bound",
            bytes.len(),
            MAX_BLOB_BYTES = misa_proto::blob::MAX_BLOB_BYTES
        ));
    }
    match source.put(bytes.to_vec(), media.as_deref()).await {
        Ok(blob) => {
            pending.lock().expect("the pending list is never poisoned").push(blob);
            Redirect::to("./").into_response()
        }
        Err(message) => upload_fault(&message),
    }
}

fn upload_fault(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Html(format!("<p class=\"n-notice.error\">{}</p>", escape(message))),
    )
        .into_response()
}

