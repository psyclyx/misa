//! Web search, as a tool.
//!
//! Three backends, and the reason there are three is the reason there should be a seam:
//! they disagree about almost everything. Brave takes a subscription token in a header
//! and answers with a nested `web.results`; Tavily takes its key in the request *body* and
//! answers flat; SearXNG is self-hosted and needs no credential at all, which is why it is
//! the default — a search tool that cannot run without an account is a search tool most
//! people cannot use.
//!
//! The tool is a capability, so it may read a credential directly. That is deliberate and
//! it is not a hole: this code is in the kernel, not in a plugin. A policy asks for
//! `tool.run { name: "web_search" }` and never learns which key was used or whether there
//! was one.

use std::sync::Arc;

use async_trait::async_trait;
use misa_value::Value;

use crate::http::Http;
use crate::{SearchBackend, SearchKind, Tool};

pub struct WebSearch {
    http: Arc<Http>,
    backend: SearchBackend,
}

impl WebSearch {
    pub fn new(http: Arc<Http>, backend: SearchBackend) -> WebSearch {
        WebSearch { http, backend }
    }

    fn query(args: &Value) -> Result<String, String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .or_else(|| args.as_str())
            .unwrap_or_default()
            .to_string();
        if query.trim().is_empty() {
            return Err("no `query` was given".into());
        }
        Ok(query)
    }

    async fn brave(&self, query: &str) -> Result<Vec<Hit>, String> {
        let url = format!(
            "{}/res/v1/web/search?q={}&count={}",
            self.backend.base_url.trim_end_matches('/'),
            urlencode(query),
            self.backend.limit
        );
        let mut request = crate::http::Request::get(url).header("accept", "application/json");
        if let Some(slot) = &self.backend.credential {
            request =
                request.with_credential(crate::Credential::header(slot, "x-subscription-token"));
        }
        let response = self.http.send(&request).await?;
        if !response.ok() {
            return Err(format!(
                "brave answered {}: {}",
                response.status,
                truncate(&response.text(), 400)
            ));
        }
        let body = response
            .json()
            .ok_or_else(|| "brave answered with something that is not json".to_string())?;
        Ok(parse_hits(
            body.pointer("/web/results"),
            &["title", "url"],
            &["description"],
        ))
    }

    async fn tavily(&self, query: &str) -> Result<Vec<Hit>, String> {
        // Tavily wants its key in the body, which is the one backend where a credential
        // cannot be a header. It is read here, in the kernel, and never handed out.
        let key = match &self.backend.credential {
            Some(slot) => self.http.credentials.secret(slot).ok_or_else(|| {
                format!("there is no credential for `{slot}`; add one before searching")
            })?,
            None => return Err("tavily needs a credential".into()),
        };
        let body = serde_json::json!({
            "api_key": key,
            "query": query,
            "max_results": self.backend.limit,
        })
        .to_string();
        let url = format!("{}/search", self.backend.base_url.trim_end_matches('/'));
        let response = self
            .http
            .send(
                &crate::http::Request::post(url, body.into_bytes())
                    .header("accept", "application/json"),
            )
            .await?;
        if !response.ok() {
            return Err(format!(
                "tavily answered {}: {}",
                response.status,
                truncate(&response.text(), 400)
            ));
        }
        let parsed = response
            .json()
            .ok_or_else(|| "tavily answered with something that is not json".to_string())?;
        Ok(parse_hits(
            parsed.get("results"),
            &["title", "url"],
            &["content"],
        ))
    }

    async fn searxng(&self, query: &str) -> Result<Vec<Hit>, String> {
        let url = format!(
            "{}/search?q={}&format=json",
            self.backend.base_url.trim_end_matches('/'),
            urlencode(query)
        );
        let response = self
            .http
            .send(&crate::http::Request::get(url).header("accept", "application/json"))
            .await?;
        if !response.ok() {
            return Err(format!(
                "searxng answered {}: {}",
                response.status,
                truncate(&response.text(), 400)
            ));
        }
        let parsed = response
            .json()
            .ok_or_else(|| "searxng answered with something that is not json".to_string())?;
        Ok(parse_hits(
            parsed.get("results"),
            &["title", "url"],
            &["content", "snippet"],
        ))
    }

    /// Whether this backend can run at all, for a diagnostic before a call.
    pub fn ready(&self) -> Result<(), String> {
        match (&self.backend.kind, &self.backend.credential) {
            (SearchKind::Searxng, _) => Ok(()),
            (_, Some(slot)) if self.http.credentials.has(slot) => Ok(()),
            (_, Some(slot)) => Err(format!("there is no credential for `{slot}`")),
            (kind, None) => Err(format!("`{}` needs a credential slot", kind.id())),
        }
    }
}

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    async fn run(&self, args: &Value) -> Result<String, String> {
        let query = match WebSearch::query(args) {
            Ok(query) => query,
            // A missing argument is a result the model can act on.
            Err(reason) => return Ok(reason),
        };
        let limit = args
            .get("limit")
            .and_then(Value::as_i64)
            .map(|limit| limit.clamp(1, 20) as usize)
            .unwrap_or(self.backend.limit.min(10) as usize);

        let result = match self.backend.kind {
            SearchKind::Brave => self.brave(&query).await,
            SearchKind::Tavily => self.tavily(&query).await,
            SearchKind::Searxng => self.searxng(&query).await,
        };
        let hits = match result {
            Ok(hits) => hits,
            Err(message) => return Ok(format!("the search failed: {message}")),
        };
        if hits.is_empty() {
            return Ok(format!("no results for `{query}`"));
        }
        let mut out = format!("{} results for `{query}`\n", hits.len().min(limit));
        for (index, hit) in hits.iter().take(limit).enumerate() {
            out.push_str(&format!("{}. {}\n   {}\n", index + 1, hit.title, hit.url));
            if !hit.snippet.is_empty() {
                out.push_str(&format!("   {}\n", truncate(&hit.snippet, 400)));
            }
        }
        Ok(out)
    }
}

/// One result, from whichever shape it arrived in.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Read results out of a list, looking for one of several possible key names.
///
/// Several, because the three backends disagree and a fourth will disagree again: this
/// is where the disagreement is absorbed so nothing above it has to know.
fn parse_hits(
    value: Option<&serde_json::Value>,
    title_keys: &[&str],
    snippet_keys: &[&str],
) -> Vec<Hit> {
    let Some(items) = value.and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let pick = |keys: &[&str]| -> String {
                keys.iter()
                    .find_map(|key| item.get(*key).and_then(|value| value.as_str()))
                    .unwrap_or_default()
                    .to_string()
            };
            let url = item
                .get("url")
                .or_else(|| item.get("link"))
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            if url.is_empty() {
                return None;
            }
            Some(Hit {
                title: pick(title_keys),
                url,
                snippet: pick(snippet_keys),
            })
        })
        .collect()
}

/// A query string, escaped. Small enough to own rather than pull a dependency for.
pub fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::Credentials;

    /// One request, one canned response, hand-written so the wire is what is tested.
    async fn serve(body: &str, status: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let body = body.to_string();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 8192];
            let _ = stream.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
        });
        address
    }

    fn backend(kind: SearchKind, base_url: String) -> SearchBackend {
        SearchBackend {
            kind,
            base_url,
            credential: None,
            limit: 5,
        }
    }

    #[tokio::test]
    async fn a_searxng_instance_needs_no_credential_at_all() {
        let body = r#"{"results":[{"title":"Rust","url":"https://rust-lang.org","content":"a language"}]}"#;
        let address = serve(body, "200 OK").await;
        let http = Arc::new(Http::new(Arc::new(Credentials::in_memory())).unwrap());
        let tool = WebSearch::new(
            http,
            backend(SearchKind::Searxng, format!("http://{address}")),
        );
        assert!(tool.ready().is_ok());
        let out = tool
            .run(&Value::map([("query", Value::str("rust"))]))
            .await
            .unwrap();
        assert!(out.contains("https://rust-lang.org"), "{out}");
        assert!(out.contains("a language"), "{out}");
    }

    #[tokio::test]
    async fn a_brave_result_is_read_out_of_its_nested_shape() {
        let body = r#"{"web":{"results":[{"title":"Rust","url":"https://rust-lang.org","description":"fast"}]}}"#;
        let address = serve(body, "200 OK").await;
        let http = Arc::new(Http::new(Arc::new(Credentials::in_memory())).unwrap());
        let mut configuration = backend(SearchKind::Brave, format!("http://{address}"));
        configuration.credential = Some("search.brave".into());
        let tool = WebSearch::new(http, configuration);
        // With no credential it says so before it makes a request.
        assert!(tool.ready().is_err());
        let out = tool
            .run(&Value::map([("query", Value::str("rust"))]))
            .await
            .unwrap();
        assert!(out.contains("the search failed"), "{out}");
    }

    #[tokio::test]
    async fn a_refusal_is_reported_with_its_status_rather_than_parsed() {
        let address = serve(r#"{"error":"rate limited"}"#, "429 Too Many Requests").await;
        let http = Arc::new(Http::new(Arc::new(Credentials::in_memory())).unwrap());
        let tool = WebSearch::new(
            http,
            backend(SearchKind::Searxng, format!("http://{address}")),
        );
        let out = tool
            .run(&Value::map([("query", Value::str("rust"))]))
            .await
            .unwrap();
        assert!(out.contains("429"), "{out}");
    }

    #[tokio::test]
    async fn a_query_that_was_not_given_is_a_result_and_not_a_request() {
        let http = Arc::new(Http::new(Arc::new(Credentials::in_memory())).unwrap());
        let tool = WebSearch::new(
            http,
            backend(SearchKind::Searxng, "http://127.0.0.1:1".into()),
        );
        let out = tool.run(&Value::Null).await.unwrap();
        assert!(out.contains("no `query`"), "{out}");
    }

    #[test]
    fn a_query_is_escaped_so_a_space_cannot_end_it() {
        assert_eq!(urlencode("a b"), "a+b");
        assert_eq!(urlencode("c++"), "c%2B%2B");
        assert_eq!(urlencode("ünï"), "%C3%BCn%C3%AF");
    }

    #[test]
    fn results_without_a_url_are_dropped_because_nothing_can_be_opened() {
        let parsed: serde_json::Value = serde_json::from_str(
            r#"[{"title":"no url","content":"x"},{"title":"ok","url":"https://example.invalid","content":"y"}]"#,
        )
        .unwrap();
        let hits = parse_hits(Some(&parsed), &["title"], &["content"]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://example.invalid");
    }
}
