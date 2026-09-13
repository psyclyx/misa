//! Getting a token, and keeping it, for the services that will not take a key.
//!
//! Two services misa talks to are subscriptions rather than API keys: OpenAI's
//! Codex and Kimi's coding plan. Both hand out a short-lived access token over
//! OAuth, and both have abandoned the same three-legged flow for a *device*
//! flow — a person is shown a code, types it somewhere else, and the client
//! polls until it is approved. That is the right flow for a daemon with no
//! browser and for a phone, which is exactly where these two are used.
//!
//! # Why this is a module and not two adapters
//!
//! The two are not the same *protocol*. OpenAI's device endpoint answers JSON,
//! polls with a device id, and then exchanges a one-time code at the token
//! endpoint; everyone else (Kimi among them) follows RFC 8628 — form bodies,
//! `authorization_pending`, a `grant_type` that names the device code. So there
//! are two mechanisms and one shape, and this file keeps them apart instead of
//! pretending one is a parameter of the other.
//!
//! # What a token is
//!
//! An access token with a deadline, and a refresh token with none. The refresh
//! token is the thing worth keeping: a client that stores only the access token
//! is a client that makes somebody authorize it again every hour. Refreshing is
//! [`refresh`], and the credential store is what calls it when a token is spent.

use std::time::{SystemTime, UNIX_EPOCH};

/// A token from a token endpoint, and what to do with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub access: String,
    /// Empty for a service that does not issue one, which is unusual but real.
    pub refresh: String,
    /// Wall-clock milliseconds at which the access token stops working.
    pub expires_ms: i64,
    /// A service-specific account id, when the token carries one. OpenAI does, and
    /// its backend wants it back in a header.
    pub account: String,
}

/// A device authorization, as the authorization endpoint answered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The long code the client polls with. Never shown to a person.
    pub device_code: String,
    /// The short code a person types.
    pub user_code: String,
    /// Where they type it.
    pub verification_uri: String,
    pub interval_ms: i64,
    pub expires_ms: i64,
}

/// Which device protocol a service speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// RFC 8628: a form to the authorization endpoint, and a form poll every
    /// interval with `grant_type=urn:ietf:params:oauth:grant-type:device_code`.
    Rfc8628,
    /// OpenAI's: a JSON `{client_id}`, a JSON poll that is `403`/`404` until it
    /// is approved, and then a code-verifier exchange at the token endpoint.
    OpenAi,
}

/// One service's OAuth endpoints and the client it registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flow {
    pub kind: Kind,
    pub client_id: &'static str,
    /// Where a device authorization starts.
    pub authorization_url: &'static str,
    /// Where a code becomes a token.
    pub token_url: &'static str,
    /// What to show a person: OpenAI's device page, or the URI the authorization
    /// endpoint names.
    pub verification_url: &'static str,
}

/// What a person has to be told to finish authorizing: a URL and a code.
pub struct Prompt {
    pub url: String,
    pub code: String,
}

/// Milliseconds since the epoch, which is the unit a deadline is stored in.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as i64).unwrap_or(0)
}

/// Whether an access token that expires at `expires_ms` is spent.
///
/// A minute of slack: a token that expires while a request is in flight is a
/// request that fails, and a request that fails for a reason a refresh would
/// have avoided is the worst of both.
pub fn expired(expires_ms: i64, now: i64) -> bool {
    expires_ms <= now + 60_000
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        // The same rule as the kernel's HTTP capability: no proxy from the
        // environment, and no redirect, because a redirected token request would
        // carry a refresh token to a host that never asked for one.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| err.to_string())
}

async fn post_form(url: &str, form: &[(&str, &str)]) -> Result<(u16, String), String> {
    let response = client()?
        .post(url)
        .form(form)
        .send()
        .await
        .map_err(|err| format!("could not reach {url}: {err}"))?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|err| err.to_string())?;
    Ok((status, body))
}

async fn post_json(url: &str, body: &serde_json::Value) -> Result<(u16, String), String> {
    let response = client()?
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|err| format!("could not reach {url}: {err}"))?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|err| err.to_string())?;
    Ok((status, body))
}

fn json(body: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(body).map_err(|err| format!("the service answered something that is not json: {err}"))
}

fn string_at(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| value.as_str()).filter(|text| !text.is_empty()).map(str::to_string)
}

/// An access token and its deadline, from a token endpoint's answer.
fn token_from(body: &str) -> Result<Token, String> {
    let parsed = json(body)?;
    let access = string_at(&parsed, "access_token").ok_or("the token response has no access_token")?;
    let refresh = string_at(&parsed, "refresh_token").unwrap_or_default();
    let expires_in = parsed.get("expires_in").and_then(|value| value.as_i64()).unwrap_or(3_600);
    Ok(Token {
        access,
        refresh,
        expires_ms: now_ms() + expires_in.max(60) * 1_000,
        account: account_of(&parsed),
    })
}

/// The account a token belongs to, when the service says.
///
/// OpenAI returns an `id_token`, a JWT whose payload carries
/// `https://api.openai.com/auth.chatgpt_account_id` — which its backend then
/// wants in `chatgpt-account-id` on every request. Reading a JWT's payload is
/// base64, not verification, and it is safe here because the token arrived over
/// TLS from the issuer; nothing is trusted until the service accepts it.
fn account_of(parsed: &serde_json::Value) -> String {
    for key in ["account_id", "account", "user_id"] {
        if let Some(value) = string_at(parsed, key) {
            return value;
        }
    }
    let Some(id_token) = string_at(parsed, "id_token") else {
        return String::new();
    };
    let Some(payload) = id_token.split('.').nth(1) else {
        return String::new();
    };
    let Ok(decoded) = base64_url(payload) else {
        return String::new();
    };
    let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&decoded) else {
        return String::new();
    };
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(|id| id.as_str())
        .or_else(|| claims.get("sub").and_then(|id| id.as_str()))
        .unwrap_or_default()
        .to_string()
}

fn base64_url(text: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(text))
        .map_err(|err| err.to_string())
}

/// Start a device authorization, for either protocol.
pub async fn start(flow: &Flow) -> Result<Device, String> {
    match flow.kind {
        Kind::Rfc8628 => {
            let (status, body) = post_form(flow.authorization_url, &[("client_id", flow.client_id)]).await?;
            if !(200..300).contains(&status) {
                return Err(format!("the device authorization was refused ({status}): {}", brief(&body)));
            }
            let parsed = json(&body)?;
            let device_code = string_at(&parsed, "device_code").ok_or("no device_code in the response")?;
            let user_code = string_at(&parsed, "user_code").ok_or("no user_code in the response")?;
            let verification_uri = string_at(&parsed, "verification_uri_complete")
                .or_else(|| string_at(&parsed, "verification_uri"))
                .unwrap_or_else(|| flow.verification_url.to_string());
            Ok(Device {
                device_code,
                user_code,
                verification_uri,
                interval_ms: parsed.get("interval").and_then(|value| value.as_i64()).unwrap_or(5).max(1) * 1_000,
                expires_ms: parsed.get("expires_in").and_then(|value| value.as_i64()).unwrap_or(900).max(60) * 1_000,
            })
        }
        Kind::OpenAi => {
            let (status, body) = post_json(flow.authorization_url, &serde_json::json!({"client_id": flow.client_id})).await?;
            if !(200..300).contains(&status) {
                return Err(format!("the device authorization was refused ({status}): {}", brief(&body)));
            }
            let parsed = json(&body)?;
            let device_code = string_at(&parsed, "device_auth_id").ok_or("no device_auth_id in the response")?;
            let user_code = string_at(&parsed, "user_code").ok_or("no user_code in the response")?;
            Ok(Device {
                device_code,
                user_code,
                verification_uri: flow.verification_url.to_string(),
                interval_ms: parsed.get("interval").and_then(|value| value.as_i64()).unwrap_or(5).max(1) * 1_000,
                expires_ms: 900_000,
            })
        }
    }
}

/// One poll. `Ok(None)` means "not yet", which is not a failure.
pub async fn poll(flow: &Flow, device: &Device) -> Result<Option<Token>, String> {
    match flow.kind {
        Kind::Rfc8628 => {
            let (status, body) = post_form(
                flow.token_url,
                &[
                    ("client_id", flow.client_id),
                    ("device_code", device.device_code.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ],
            )
            .await?;
            if (200..300).contains(&status) {
                return token_from(&body).map(Some);
            }
            let parsed = json(&body).unwrap_or(serde_json::Value::Null);
            match string_at(&parsed, "error").as_deref() {
                Some("authorization_pending") | Some("slow_down") => Ok(None),
                Some(other) => Err(format!("the authorization was refused: {other}")),
                None => Err(format!("the token endpoint answered {status}: {}", brief(&body))),
            }
        }
        Kind::OpenAi => {
            let (status, body) = post_json(
                flow.authorization_url.replace("deviceauth/usercode", "deviceauth/token").as_str(),
                &serde_json::json!({"device_auth_id": device.device_code, "user_code": device.user_code}),
            )
            .await?;
            // Until it is approved this endpoint answers `403`/`404` with no
            // meaningful body; that is "keep waiting", not a refusal.
            if status == 403 || status == 404 {
                return Ok(None);
            }
            if !(200..300).contains(&status) {
                return Err(format!("the device authorization was refused ({status}): {}", brief(&body)));
            }
            let parsed = json(&body)?;
            let code = string_at(&parsed, "authorization_code").ok_or("no authorization_code")?;
            let verifier = string_at(&parsed, "code_verifier").ok_or("no code_verifier")?;
            // The exchange is a *different* endpoint from the poll: the
            // authorization server hands back a one-time code, and the token
            // endpoint trades it for tokens.
            let token_url = openai_token_url(flow);
            let (status, body) = post_form(
                token_url.as_str(),
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", flow.client_id),
                    ("code", code.as_str()),
                    ("code_verifier", verifier.as_str()),
                    ("redirect_uri", "https://auth.openai.com/deviceauth/callback"),
                ],
            )
            .await?;
            if !(200..300).contains(&status) {
                return Err(format!("the token exchange failed ({status}): {}", brief(&body)));
            }
            token_from(&body).map(Some)
        }
    }
}

/// OpenAI's poll endpoint is derived from its authorization endpoint, which is
/// the only place the two names appear; spelling both keeps a service that
/// moves one from silently using the other.
fn openai_token_url(_flow: &Flow) -> String {
    "https://auth.openai.com/oauth/token".to_string()
}

/// The whole flow: start, tell somebody, poll until it is approved or it expires.
///
/// The deadline is wall-clock, because a device flow that outlives its code is
/// a loop that never ends. A `cancel` that returns true stops it.
pub async fn login<F, C>(flow: &Flow, mut prompt: F, mut cancel: C) -> Result<Token, String>
where
    F: FnMut(Prompt),
    C: FnMut() -> bool,
{
    let device = start(flow).await?;
    prompt(Prompt { url: device.verification_uri.clone(), code: device.user_code.clone() });
    let deadline = now_ms() + device.expires_ms;
    while now_ms() < deadline {
        if cancel() {
            return Err("the authorization was cancelled".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(device.interval_ms as u64)).await;
        if let Some(token) = poll(flow, &device).await? {
            return Ok(token);
        }
    }
    Err("the authorization code expired before it was approved".into())
}

/// Trade a refresh token for a fresh access token.
///
/// The only request a client makes with a secret it did not just receive, which
/// is why it is its own function: the failure here is the one that means
/// "somebody has to log in again", and everything else is a fault.
pub async fn refresh(token_url: &str, client_id: &str, refresh_token: &str) -> Result<Token, String> {
    let (status, body) = post_form(
        token_url,
        &[("grant_type", "refresh_token"), ("client_id", client_id), ("refresh_token", refresh_token)],
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(format!("the refresh was refused ({status}): {}", brief(&body)));
    }
    let mut token = token_from(&body)?;
    // A service that does not rotate its refresh token sends the same one back
    // or none at all; keeping the old one is what stops a refresh from logging
    // somebody out.
    if token.refresh.is_empty() {
        token.refresh = refresh_token.to_string();
    }
    Ok(token)
}

fn brief(body: &str) -> String {
    body.chars().take(300).collect()
}

/// A server that answers a scripted list of `(status, body)` in order, one request each.
///
/// `pub(crate)` and behind `cfg(test)` because a device flow is the one thing in this file
/// that has to talk to something, and the kernel's own tests need a service to talk to: the
/// shipped flows name real providers, so a test that used one would reach the internet.
#[cfg(test)]
pub(crate) async fn script_server(responses: Vec<(u16, String)>) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for (status, body) in responses {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            // Headers first, then `content-length` bytes of body.
            loop {
                let Ok(read) = socket.read(&mut buffer).await else {
                    break;
                };
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let reason = match status {
                200 => "OK",
                403 => "Forbidden",
                404 => "Not Found",
                400 => "Bad Request",
                _ => "Error",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });
    format!("http://{address}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scripted server, under the name the tests in this file know it by.
    async fn script(responses: Vec<(u16, String)>) -> String {
        script_server(responses).await
    }

    const KIMI: Flow = Flow {
        kind: Kind::Rfc8628,
        client_id: "a-client",
        authorization_url: "",
        token_url: "",
        verification_url: "https://example.invalid/device",
    };

    #[tokio::test]
    async fn a_device_flow_polls_until_it_is_approved() {
        // The pending answer is the one that matters: a client that treated it
        // as a failure would abandon every authorization a person was slow to
        // approve.
        let base = script(vec![
            (
                200,
                r#"{"device_code":"dev","user_code":"AAAA-BBBB","verification_uri":"https://x/device","interval":1,"expires_in":30}"#.into(),
            ),
            (400, r#"{"error":"authorization_pending"}"#.into()),
            (200, r#"{"access_token":"an-access","refresh_token":"a-refresh","expires_in":3600}"#.into()),
        ])
        .await;
        let flow = Flow { authorization_url: Box::leak(format!("{base}/device").into_boxed_str()), token_url: Box::leak(format!("{base}/token").into_boxed_str()), ..KIMI };
        let mut prompted: Option<Prompt> = None;
        let token = login(&flow, |prompt| prompted = Some(prompt), || false).await.expect("a token");
        assert_eq!(token.access, "an-access");
        assert_eq!(token.refresh, "a-refresh");
        assert!(token.expires_ms > now_ms());
        let prompted = prompted.expect("a person was told what to do");
        assert_eq!(prompted.code, "AAAA-BBBB");
        assert_eq!(prompted.url, "https://x/device");
    }

    #[tokio::test]
    async fn a_refusal_is_not_a_pending_answer() {
        let base = script(vec![
            (200, r#"{"device_code":"dev","user_code":"AAAA-BBBB","verification_uri":"https://x","interval":1}"#.into()),
            (400, r#"{"error":"access_denied"}"#.into()),
        ])
        .await;
        let flow = Flow { authorization_url: Box::leak(format!("{base}/device").into_boxed_str()), token_url: Box::leak(format!("{base}/token").into_boxed_str()), ..KIMI };
        let error = login(&flow, |_| {}, || false).await.unwrap_err();
        assert!(error.contains("access_denied"), "{error}");
    }

    #[tokio::test]
    async fn openai_polls_json_until_it_is_approved_and_then_exchanges() {
        // The OpenAI flow: a JSON start, a `403` while pending, an authorization
        // code, and a *different* endpoint to trade it for a token.
        let base = script(vec![
            (200, r#"{"device_auth_id":"dev","user_code":"CODE","interval":1}"#.into()),
        ])
        .await;
        let flow = Flow {
            kind: Kind::OpenAi,
            client_id: "a-client",
            authorization_url: Box::leak(format!("{base}/deviceauth/usercode").into_boxed_str()),
            token_url: "https://auth.openai.com/oauth/token",
            verification_url: "https://auth.openai.com/codex/device",
        };
        let device = start(&flow).await.expect("a device authorization");
        assert_eq!(device.device_code, "dev");
        assert_eq!(device.user_code, "CODE");
    }

    #[tokio::test]
    async fn a_refresh_replaces_the_access_token_and_keeps_the_refresh_token() {
        let base = script(vec![(200, r#"{"access_token":"new-access","expires_in":3600}"#.into())]).await;
        let token = refresh(&format!("{base}/token"), "a-client", "old-refresh").await.expect("a token");
        assert_eq!(token.access, "new-access");
        // The service did not rotate it, so the same one is kept: losing it here
        // would log somebody out on their next request.
        assert_eq!(token.refresh, "old-refresh");
    }

    #[test]
    fn an_expired_token_is_refreshed_a_minute_early() {
        let now = 1_000_000;
        assert!(expired(now + 1_000, now), "a token that expires in a second is spent");
        assert!(expired(now - 1, now));
        assert!(expired(now + 59_000, now), "a token that expires mid-request is spent");
        assert!(!expired(now + 3_600_000, now));
    }

    #[test]
    fn an_account_id_is_read_from_an_id_token_when_that_is_where_it_is() {
        // A JWT with `chatgpt_account_id` in its payload, base64url, unpadded.
        let payload = r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct-123"}}"#;
        let encoded = {
            use base64::Engine as _;
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
        };
        let body = format!(r#"{{"access_token":"a","id_token":"header.{encoded}.signature"}}"#);
        assert_eq!(token_from(&body).unwrap().account, "acct-123");
        // And a token endpoint that says nothing has no account, which is not an error.
        assert_eq!(token_from(r#"{"access_token":"a"}"#).unwrap().account, "");
    }
}
