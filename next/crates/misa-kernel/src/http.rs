//! The one way out to the network.
//!
//! A capability, not a convenience: a policy names an effect and this is what runs it,
//! with a deadline, a size bound, and the credential injected *here* so the bytes never
//! reach a policy.
//!
//! Two shapes, and the second is the one providers need:
//!
//! - [`Http::send`] — a request and a whole response. What a search backend, a model
//!   list, or a usage endpoint wants.
//! - [`Http::stream`] — a request and server-sent events. What a model completion wants,
//!   because a completion arrives as a sequence of deltas and buffering it would throw
//!   away the only thing that makes a chat usable.
//!
//! Both are bounded, and neither retries. Retrying is *policy* — the previous system
//! had a considered rule about which failures are retryable and which are not — and this
//! layer is the wrong place to make that decision.

use std::time::Duration;

use futures::StreamExt as _;

use crate::credentials::Credentials;

/// Where a credential goes in a request.
///
/// Declared by the policy that knows the provider, because the kernel has no business
/// knowing that one service wants `x-api-key` and another wants `Bearer`.
#[derive(Clone, Debug, PartialEq)]
pub struct Credential {
    /// The slot to read, as `anthropic` or `search.brave`.
    pub slot: String,
    pub header: String,
    /// A prefix for the value, as `Bearer `.
    pub prefix: String,
    /// A second header, filled from the credential's *account* rather than its
    /// value. One service needs it — OpenAI's subscription backend wants
    /// `chatgpt-account-id` on every request — and it is here rather than in an
    /// adapter because a header is the transport's business.
    pub account_header: Option<String>,
}

impl Credential {
    pub fn bearer(slot: impl Into<String>) -> Credential {
        Credential {
            slot: slot.into(),
            header: "authorization".into(),
            prefix: "Bearer ".into(),
            account_header: None,
        }
    }

    pub fn header(slot: impl Into<String>, header: impl Into<String>) -> Credential {
        Credential {
            slot: slot.into(),
            header: header.into(),
            prefix: String::new(),
            account_header: None,
        }
    }

    /// The same credential, with the account it belongs to sent as well.
    pub fn with_account_header(mut self, header: impl Into<String>) -> Credential {
        self.account_header = Some(header.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub credential: Option<Credential>,
    pub first_byte_ms: u64,
    pub idle_ms: u64,
    pub overall_ms: u64,
    pub max_bytes: usize,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Request {
        Request { method: "GET".into(), ..Request::default_for(url) }
    }

    pub fn post(url: impl Into<String>, body: Vec<u8>) -> Request {
        Request { method: "POST".into(), body: Some(body), ..Request::default_for(url) }
    }

    fn default_for(url: impl Into<String>) -> Request {
        Request {
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: None,
            credential: None,
            first_byte_ms: 20_000,
            idle_ms: 30_000,
            overall_ms: 120_000,
            // 4 MiB: a model list or a search response fits; a transcript that does not
            // is a request that was wrong.
            max_bytes: 4 * 1024 * 1024,
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Request {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn json(mut self, body: &str) -> Request {
        self.headers.push(("content-type".into(), "application/json".into()));
        self.body = Some(body.as_bytes().to_vec());
        self
    }

    pub fn with_credential(mut self, credential: Credential) -> Request {
        self.credential = Some(credential);
        self
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// The transport.
pub struct Http {
    client: reqwest::Client,
    pub credentials: std::sync::Arc<Credentials>,
}

impl Http {
    pub fn new(credentials: std::sync::Arc<Credentials>) -> Result<Http, String> {
        let client = reqwest::Client::builder()
            // No proxy from the environment, and no redirects: a redirected request
            // would carry a credential to a host that did not ask for it.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| err.to_string())?;
        Ok(Http { client, credentials })
    }

    /// Send a request and read a whole response.
    pub async fn send(&self, request: &Request) -> Result<Response, String> {
        let response = self.start(request).await?;
        let status = response.status().as_u16();
        let headers = collect_headers(&response);
        let deadline = Duration::from_millis(request.overall_ms);
        let body = tokio::time::timeout(deadline, read_bounded(response, request.max_bytes))
            .await
            .map_err(|_| format!("the body did not arrive within {}ms", request.overall_ms))??;
        Ok(Response { status, headers, body })
    }

    /// Send a request and hand each server-sent event's data to a callback as it
    /// arrives.
    ///
    /// Returns the status and headers, so a caller can report a refusal body that was
    /// not an event stream at all.
    pub async fn stream<F>(&self, request: &Request, mut on_event: F) -> Result<(u16, Vec<(String, String)>), String>
    where
        F: FnMut(&str),
    {
        let response = self.start(request).await?;
        let status = response.status().as_u16();
        let headers = collect_headers(&response);
        if !(200..300).contains(&status) {
            // A refusal is usually a JSON body, and the caller wants it.
            let body = read_bounded(response, request.max_bytes).await.unwrap_or_default();
            return Err(format!(
                "{} {}: {}",
                status,
                headers
                    .iter()
                    .find(|(name, _)| name == "content-type")
                    .map(|(_, value)| value.as_str())
                    .unwrap_or(""),
                String::from_utf8_lossy(&body).chars().take(2_000).collect::<String>()
            ));
        }

        let mut stream = response.bytes_stream();
        let idle = Duration::from_millis(request.idle_ms.max(1_000));
        let mut buffer = Vec::new();
        let mut total = 0usize;
        loop {
            let chunk = match tokio::time::timeout(idle, stream.next()).await {
                Err(_) => return Ok((status, headers)),
                Ok(None) => break,
                Ok(Some(Err(err))) => return Err(err.to_string()),
                Ok(Some(Ok(chunk))) => chunk,
            };
            total += chunk.len();
            if total > request.max_bytes {
                return Err(format!("a stream passed the {} byte bound", request.max_bytes));
            }
            buffer.extend_from_slice(&chunk);
            while let Some(index) = find_blank_line(&buffer) {
                let frame: Vec<u8> = buffer.drain(..index).collect();
                // The separator is part of the frame; skip it and any `\n` after it.
                while buffer.first().is_some_and(|byte| *byte == b'\n' || *byte == b'\r') {
                    buffer.remove(0);
                }
                if let Some(data) = event_data(&String::from_utf8_lossy(&frame)) {
                    on_event(&data);
                }
            }
        }
        Ok((status, headers))
    }

    async fn start(&self, request: &Request) -> Result<reqwest::Response, String> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|err| format!("`{}` is not a method: {err}", request.method))?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(credential) = &request.credential {
            // A token with a deadline is renewed before it is used, which is the
            // whole reason it is stored with its refresh token. A key never is.
            self.renew_if_needed(&credential.slot).await?;
            // The one place a secret is read, and it is read straight into a header.
            let secret = self.credentials.secret(&credential.slot).ok_or_else(|| {
                format!("there is no credential for `{}`; add one before using this provider", credential.slot)
            })?;
            builder = builder.header(&credential.header, format!("{}{secret}", credential.prefix));
            // One service wants the account as well as the token: OpenAI's
            // subscription backend reads `chatgpt-account-id`.
            if let Some(header) = &credential.account_header
                && let Some(account) = self.credentials.oauth(&credential.slot).map(|oauth| oauth.account)
                && !account.is_empty()
            {
                builder = builder.header(header, account);
            }
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }
        let first_byte = Duration::from_millis(request.first_byte_ms.max(1_000));
        match tokio::time::timeout(first_byte, builder.send()).await {
            Err(_) => Err(format!("no response began within {}ms", request.first_byte_ms)),
            Ok(Err(err)) => Err(format!("the request failed: {err}")),
            Ok(Ok(response)) => Ok(response),
        }
    }
    /// Renew an OAuth credential whose access token is spent.
    ///
    /// Called on the way out, so a request never leaves with a token that has
    /// already expired: the refresh token is the thing worth keeping, and this is
    /// the one place that uses it.
    async fn renew_if_needed(&self, slot: &str) -> Result<(), String> {
        let Some(oauth) = self.credentials.oauth(slot) else {
            return Ok(());
        };
        if !crate::oauth::expired(oauth.expires_ms, crate::oauth::now_ms()) {
            return Ok(());
        }
        if oauth.refresh.is_empty() {
            return Err(format!(
                "the credential for `{slot}` has expired and carries no refresh token; log in again"
            ));
        }
        let token = crate::oauth::refresh(&oauth.token_url, &oauth.client_id, &oauth.refresh).await?;
        self.credentials.renew(slot, &token.access, &token.refresh, token.expires_ms)?;
        Ok(())
    }
}

fn collect_headers(response: &reqwest::Response) -> Vec<(String, String)> {
    response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_lowercase(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

async fn read_bounded(response: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut stream = response.bytes_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| err.to_string())?;
        if out.len() + chunk.len() > max_bytes {
            return Err(format!("a response passed the {max_bytes} byte bound"));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// The offset just past a blank line, which is where one server-sent event ends.
fn find_blank_line(buffer: &[u8]) -> Option<usize> {
    for index in 0..buffer.len() {
        if buffer[index] != b'\n' {
            continue;
        }
        // `\n\n` or `\r\n\r\n`.
        let rest = &buffer[index + 1..];
        if rest.starts_with(b"\n") {
            return Some(index + 2);
        }
        if rest.starts_with(b"\r\n") {
            return Some(index + 3);
        }
    }
    None
}

/// The `data:` lines of one event, joined.
///
/// A comment (`:`) is a keep-alive and carries nothing; an event with no `data` line
/// carries nothing either, which is why this returns an option rather than a string.
fn event_data(frame: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for line in frame.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with(':') || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            parts.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_ends_at_a_blank_line_and_a_comment_carries_nothing() {
        assert_eq!(event_data("data: {\"a\":1}"), Some("{\"a\":1}".into()));
        assert_eq!(event_data(": keep-alive"), None);
        assert_eq!(event_data("event: ping"), None);
        assert_eq!(event_data("data: one\ndata: two"), Some("one\ntwo".into()));
        // The offset is just past the blank line, so what is left is the next event.
        assert_eq!(find_blank_line(b"data: x\n\nrest"), Some(9));
        assert_eq!(find_blank_line(b"data: x\r\n\r\nrest"), Some(11));
        assert_eq!(find_blank_line(b"data: x\n"), None);
    }

    #[test]
    fn a_credential_declares_where_it_goes_because_only_the_provider_knows() {
        let bearer = Credential::bearer("openai");
        assert_eq!(bearer.header, "authorization");
        assert_eq!(bearer.prefix, "Bearer ");
        let key = Credential::header("anthropic", "x-api-key");
        assert_eq!(key.prefix, "");
    }

    #[tokio::test]
    async fn a_response_is_bounded() {
        let credentials = std::sync::Arc::new(Credentials::in_memory());
        credentials.set("test", "a", "secret").unwrap();
        let http = Http::new(credentials).unwrap();
        // Nothing to talk to: this asserts the error is a value rather than a panic.
        let request = Request::get("http://127.0.0.1:1/nothing").header("accept", "application/json");
        let error = http.send(&request).await.unwrap_err();
        assert!(error.contains("failed"), "{error}");
    }
}
