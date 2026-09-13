//! Model adapters.
//!
//! A provider is the seam the previous system called `llm`: what a completion request
//! and its stream look like on the wire, and how to turn them into the deltas a session
//! handles. Three real ones ship, because there are three shapes in the world and having
//! all three proves the seam is a seam:
//!
//! - [`OpenAiChat`] — `/chat/completions` with `stream: true`. The shape most providers
//!   speak, including most of the ones that are not OpenAI.
//! - [`AnthropicMessages`] — `/v1/messages` with content blocks and typed deltas.
//! - [`OpenAiResponses`] — `/responses`, where a message is a list of *input items*, a tool
//!   call is an item of its own, and the stream's events name the item they belong to. It is
//!   the shape a ChatGPT subscription's backend speaks, and treating it as chat completions
//!   with a flag is how an answer never arrives.
//!
//! None is a mock and none is complete: there is no retry (that is policy), no prompt
//! caching directive, and no image *output*. What each does have is the part that cannot be
//! faked — a real request, a real server-sent event stream, and a real translation into
//! [`Answer`], including tool calls arriving in pieces and reasoning that is kept apart from
//! the answer.
//!
//! # Streaming is the only mode
//!
//! There is no `complete()` that buffers. A chat that waits for a whole answer before
//! showing any of it is worse than one that shows it as it comes, and a non-streaming
//! path would be a second implementation of every parser here.

use std::sync::Arc;

use async_trait::async_trait;
use misa_value::Value;
use tokio::sync::mpsc;

use crate::credentials::{Credentials, looks_like_a_secret};
use crate::http::{Credential, Http, Request};
use crate::presets::{Effort, Preset};
use crate::{Answer, KernelEvent, ProviderRequest, base64};

/// What one service's dialect changes about a request, as against what it shares.
///
/// There are two wire shapes here and a dozen services that speak one of them, and this is
/// the whole of the difference: the field a token budget goes in, how a reasoning effort is
/// spelled, and whether captured thinking is sent back on a field of its own. Everything else
/// — the body, the deltas, the tool calls arriving in pieces — is the protocol.
#[derive(Clone, Copy, Debug)]
pub struct Dialect {
    /// `max_tokens`, or `max_completion_tokens` for the services that only accept that one.
    pub max_tokens_field: &'static str,
    /// How a reasoning effort is spelled, and whether one may be sent at all.
    pub effort: Effort,
    /// The field captured thinking goes back on, for the services that require it on a
    /// tool-call turn. `None` means the service never sees it again: an unknown field is a
    /// 400 on most of them, and a wrong guess here is worse than no guess.
    pub reasoning_field: Option<&'static str>,
}

impl Default for Dialect {
    /// What a service nobody described here gets: the oldest and most widely accepted
    /// spelling of everything.
    fn default() -> Dialect {
        Dialect { max_tokens_field: "max_tokens", effort: Effort::Reasoning, reasoning_field: None }
    }
}

impl From<&Preset> for Dialect {
    fn from(preset: &Preset) -> Dialect {
        Dialect {
            max_tokens_field: preset.max_tokens_field,
            effort: preset.effort,
            reasoning_field: reasoning_field_of(preset.id),
        }
    }
}

/// The services that want captured thinking back on a field of its own.
///
/// DeepSeek is the one that needs it: it rejects a tool-call turn whose reasoning was not sent
/// back along with it. Every other service treats an unexpected field as a bad request, so
/// this is a list of ids rather than a guess about what a service might tolerate.
fn reasoning_field_of(id: &str) -> Option<&'static str> {
    match id {
        "deepseek" => Some("reasoning_content"),
        _ => None,
    }
}

/// What a provider does with one request.
#[async_trait]
pub trait Provider: Send + Sync {
    /// The id a session names in its composition.
    fn id(&self) -> &str;

    /// The configured API base, used by capabilities scoped to this adapter.
    fn api_base(&self) -> Option<&str> {
        None
    }

    /// Where this service lists its models, when it has such a thing.
    ///
    /// On the trait rather than in a table beside it, because the address is the adapter's own
    /// business: a provider composed against a proxy, a mirror, or a server on this machine
    /// lists its models where *that* address lists them, and a service with no such endpoint
    /// (a script, a test double) says so by returning nothing.
    fn models_url(&self) -> Option<String> {
        None
    }

    /// The credential this provider sends, by slot. Never a value, and never formatted.
    fn credential(&self) -> Option<Credential> {
        None
    }

    /// Stream a completion. Deltas are pushed as they arrive; the returned value is the
    /// whole answer, including any tool calls and usage the provider reported.
    async fn stream(
        &self,
        request: &ProviderRequest,
        id: &str,
        out: &mpsc::UnboundedSender<KernelEvent>,
    ) -> Result<Answer, String>;
}

/// A tool call as it is assembled, because both wire formats send one in pieces.
#[derive(Clone, Debug, Default)]
pub struct PartialCall {
    pub id: String,
    pub name: String,
    pub args: String,
}

impl PartialCall {
    /// A call is usable once it has a name; arguments may legitimately be empty.
    pub fn usable(&self) -> bool {
        !self.name.is_empty()
    }

    pub fn to_value(&self) -> Value {
        let args = if self.args.trim().is_empty() {
            Value::map([])
        } else {
            // Arguments arrive as a JSON string in pieces, and are an object once the
            // pieces are joined. If the whole of it is not valid JSON the model produced
            // something no tool can be given, and the string is passed through rather
            // than losing what it said.
            serde_json::from_str::<Value>(&self.args).unwrap_or_else(|_| Value::str(&self.args))
        };
        Value::map([
            ("id", Value::str(&self.id)),
            ("name", Value::str(&self.name)),
            ("args", args),
        ])
    }
}

/// Assemble the calls of a stream, keeping the order they appeared in, because a model
/// that asked for two tools meant them in that order.
#[derive(Default)]
pub struct CallBuilder {
    calls: Vec<PartialCall>,
}

impl CallBuilder {
    pub fn start(&mut self, index: usize, id: Option<&str>, name: Option<&str>) {
        while self.calls.len() <= index {
            self.calls.push(PartialCall::default());
        }
        let call = &mut self.calls[index];
        if let Some(id) = id
            && !id.is_empty()
        {
            call.id = id.to_string();
        }
        if let Some(name) = name
            && !name.is_empty()
        {
            call.name = name.to_string();
        }
    }

    pub fn args(&mut self, index: usize, fragment: &str) {
        self.start(index, None, None);
        self.calls[index].args.push_str(fragment);
    }

    /// Number the calls that arrived without one, because a result has to name the call
    /// it answers and a tool cannot be run without a name to report.
    pub fn finish(self) -> Value {
        Value::list(
            self.calls
                .into_iter()
                .enumerate()
                .filter(|(_, call)| call.usable())
                .map(|(index, mut call)| {
                    if call.id.is_empty() {
                        call.id = format!("call.{index}");
                    }
                    call.to_value()
                })
                .collect::<Vec<_>>(),
        )
    }
}

/// OpenAI's chat completions, and everything that imitates it.
pub struct OpenAiChat {
    id: String,
    base_url: String,
    /// Where this service lists its models, relative to the base url.
    models_path: &'static str,
    credential: Option<Credential>,
    http: Arc<Http>,
    /// Extra request fields, from the composition: `temperature`, and whatever else a
    /// daemon was configured with.
    options: Value,
    /// What this service's dialect changes.
    dialect: Dialect,
}

impl OpenAiChat {
    pub fn new(id: impl Into<String>, base_url: impl Into<String>, http: Arc<Http>) -> OpenAiChat {
        OpenAiChat {
            id: id.into(),
            base_url: base_url.into(),
            models_path: "models",
            credential: None,
            http,
            options: Value::Null,
            dialect: Dialect::default(),
        }
    }

    /// An adapter for one of the services this daemon knows by name.
    ///
    /// The address, the header, and the dialect all come from the preset, which is the whole
    /// reason a preset exists: adding a service is a row in one table and not an adapter.
    pub fn from_preset(preset: &Preset, http: Arc<Http>) -> OpenAiChat {
        let chat = OpenAiChat::new(preset.id, preset.base_url, http)
            .with_dialect(preset.into())
            .with_models_path(preset.models_path);
        match preset.credential() {
            Some(credential) => chat.with_credential(credential),
            None => chat,
        }
    }

    /// Where this service lists its models. Not `models` for every service — Anthropic's is
    /// `v1/models` — which is why it is asked rather than assumed.
    pub fn with_models_path(mut self, models_path: &'static str) -> OpenAiChat {
        self.models_path = models_path;
        self
    }

    /// Read the key from a slot, and send it as a bearer token.
    pub fn credentialed(mut self, slot: impl Into<String>) -> OpenAiChat {
        self.credential = Some(Credential::bearer(slot));
        self
    }

    /// A credential whose header a preset chose: `Authorization: Bearer` for most services,
    /// and whatever the one that is not most services wants.
    pub fn with_credential(mut self, credential: Credential) -> OpenAiChat {
        self.credential = Some(credential);
        self
    }

    pub fn with_dialect(mut self, dialect: Dialect) -> OpenAiChat {
        self.dialect = dialect;
        self
    }

    /// The same service at a different address: a proxy, a mirror, or a server of your own.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> OpenAiChat {
        self.base_url = base_url.into();
        self
    }

    pub fn with_options(mut self, options: Value) -> OpenAiChat {
        self.options = options;
        self
    }

    fn request(&self, request: &ProviderRequest) -> Request {
        let mut body = serde_json::Map::new();
        body.insert("model".into(), serde_json::Value::String(request.model.clone()));
        body.insert("stream".into(), serde_json::Value::Bool(true));
        // Most services report usage only when asked. A spend report that is silently zero
        // because nobody asked is worse than one that arrives in the last chunk.
        body.insert("stream_options".into(), serde_json::json!({"include_usage": true}));
        body.insert("messages".into(), openai_messages_with(&request.messages, self.dialect));
        if let Some(tools) = tools_json(&request.tools) {
            body.insert("tools".into(), tools);
        }
        // The session's settings, and the two of them whose *name* this service decides. A
        // session sets an option; an adapter knows what this service calls it.
        for (key, value) in request.settings.as_map().into_iter().flatten() {
            match key.as_str() {
                "reasoning_effort" => match (self.dialect.effort, value.as_str()) {
                    (Effort::Reasoning, Some(_)) => {
                        body.insert("reasoning_effort".into(), json_of(value));
                    }
                    // OpenRouter proxies many services and spells an effort its own way.
                    (Effort::OpenRouter, Some(level)) => {
                        body.insert("reasoning".into(), serde_json::json!({"effort": level}));
                    }
                    // Anthropic's spelling of an effort is a token budget, and this is not
                    // Anthropic: sending the word would be a 400, and sending nothing is what
                    // a service that takes no such setting expects.
                    _ => {}
                },
                "max_tokens" => {
                    body.insert(self.dialect.max_tokens_field.into(), json_of(value));
                }
                other => {
                    body.insert(other.to_string(), json_of(value));
                }
            }
        }
        if let Value::Map(options) = &self.options {
            for (key, value) in options.iter() {
                body.insert(key.clone(), json_of(value));
            }
        }
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut request = Request::post(url, serde_json::Value::Object(body).to_string().into_bytes())
            .header("accept", "text/event-stream");
        if let Some(credential) = &self.credential {
            request = request.with_credential(credential.clone());
        }
        request
    }
}

#[async_trait]
impl Provider for OpenAiChat {
    fn id(&self) -> &str {
        &self.id
    }

    fn api_base(&self) -> Option<&str> {
        Some(&self.base_url)
    }

    fn models_url(&self) -> Option<String> {
        Some(format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            self.models_path.trim_start_matches('/')
        ))
    }

    fn credential(&self) -> Option<Credential> {
        self.credential.clone()
    }

    async fn stream(
        &self,
        request: &ProviderRequest,
        id: &str,
        out: &mpsc::UnboundedSender<KernelEvent>,
    ) -> Result<Answer, String> {
        let mut answer = Answer::default();
        let mut calls = CallBuilder::default();
        let mut failure: Option<String> = None;

        let result = self
            .http
            .stream(&self.request(request), |data| {
                if data == "[DONE]" {
                    return;
                }
                let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) else {
                    return;
                };
                if let Some(error) = chunk.get("error") {
                    failure = Some(error.to_string());
                    return;
                }
                if let Some(usage) = chunk.get("usage") {
                    answer.input_tokens = usage.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    answer.output_tokens = usage.get("completion_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                }
                let Some(choice) = chunk.get("choices").and_then(|choices| choices.get(0)) else {
                    return;
                };
                let delta = choice.get("delta").or_else(|| choice.get("message"));
                let Some(delta) = delta else {
                    return;
                };
                if let Some(text) = delta.get("content").and_then(|text| text.as_str()) {
                    answer.text.push_str(text);
                    let _ = out.send(KernelEvent::ProviderDelta { id: id.to_string(), text: text.to_string() });
                }
                // Some services stream their reasoning beside the answer rather than in it:
                // DeepSeek in `reasoning_content`, OpenRouter in `reasoning`. It is not the
                // answer, and a client that shows thinking has to be able to tell them apart.
                if let Some(text) = delta
                    .get("reasoning_content")
                    .or_else(|| delta.get("reasoning"))
                    .and_then(|text| text.as_str())
                {
                    answer.thinking.push_str(text);
                    let _ = out.send(KernelEvent::ProviderThinking { id: id.to_string(), text: text.to_string() });
                }
                if let Some(list) = delta.get("tool_calls").and_then(|calls| calls.as_array()) {
                    for call in list {
                        let index = call.get("index").and_then(|index| index.as_u64()).unwrap_or(0) as usize;
                        let function = call.get("function");
                        calls.start(
                            index,
                            call.get("id").and_then(|id| id.as_str()),
                            function.and_then(|function| function.get("name")).and_then(|name| name.as_str()),
                        );
                        if let Some(fragment) = function
                            .and_then(|function| function.get("arguments"))
                            .and_then(|args| args.as_str())
                        {
                            calls.args(index, fragment);
                        }
                    }
                }
            })
            .await;

        if let Some(error) = failure {
            return Err(error);
        }
        result?;
        answer.tool_calls = calls.finish();
        Ok(answer)
    }
}

/// Anthropic's messages API: content blocks, typed deltas, and `x-api-key`.
pub struct AnthropicMessages {
    id: String,
    base_url: String,
    /// Where this service lists its models: `v1/models`, not `models`.
    models_path: &'static str,
    credential: Option<Credential>,
    http: Arc<Http>,
    options: Value,
    /// What this service's dialect changes. Anthropic's own spellings by default.
    dialect: Dialect,
}

impl AnthropicMessages {
    pub fn new(id: impl Into<String>, base_url: impl Into<String>, http: Arc<Http>) -> AnthropicMessages {
        AnthropicMessages {
            id: id.into(),
            base_url: base_url.into(),
            models_path: "v1/models",
            credential: None,
            http,
            options: Value::Null,
            dialect: Dialect {
                max_tokens_field: "max_tokens",
                effort: Effort::Thinking,
                reasoning_field: None,
            },
        }
    }

    /// An adapter for one of the services this daemon knows by name.
    pub fn from_preset(preset: &Preset, http: Arc<Http>) -> AnthropicMessages {
        let messages = AnthropicMessages::new(preset.id, preset.base_url, http)
            .with_dialect(preset.into())
            .with_models_path(preset.models_path);
        match preset.credential() {
            Some(credential) => messages.with_credential(credential),
            None => messages,
        }
    }

    /// Where this service lists its models: `/v1/models` rather than `/models`.
    pub fn with_models_path(mut self, models_path: &'static str) -> AnthropicMessages {
        self.models_path = models_path;
        self
    }

    /// Anthropic wants the key in `x-api-key`, with no prefix, and an API version
    /// header. The policy knows that; the kernel does not.
    pub fn credentialed(mut self, slot: impl Into<String>) -> AnthropicMessages {
        self.credential = Some(Credential::header(slot, "x-api-key"));
        self
    }

    pub fn with_credential(mut self, credential: Credential) -> AnthropicMessages {
        self.credential = Some(credential);
        self
    }

    pub fn with_dialect(mut self, dialect: Dialect) -> AnthropicMessages {
        self.dialect = dialect;
        self
    }

    /// The same service at a different address: a proxy, a mirror, or a server of your own.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> AnthropicMessages {
        self.base_url = base_url.into();
        self
    }

    pub fn with_options(mut self, options: Value) -> AnthropicMessages {
        self.options = options;
        self
    }

    fn request(&self, request: &ProviderRequest) -> Request {
        let mut body = serde_json::Map::new();
        body.insert("model".into(), serde_json::Value::String(request.model.clone()));
        body.insert("stream".into(), serde_json::Value::Bool(true));
        // Anthropic wants a token budget on every request, and a thinking budget has to fit
        // *inside* it: an effort here is a budget, and the answer needs room after the
        // thinking, so asking to think harder raises the ceiling rather than filling it.
        let thinking = match self.dialect.effort {
            Effort::Thinking => request
                .settings
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .map(thinking_budget),
            _ => None,
        };
        let max_tokens = request
            .settings
            .get("max_tokens")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| thinking.map_or(8_192, |budget| budget + 8_192));
        body.insert("max_tokens".into(), serde_json::Value::Number(max_tokens.into()));
        if let Some(budget) = thinking {
            body.insert(
                "thinking".into(),
                serde_json::json!({"type": "enabled", "budget_tokens": budget}),
            );
        }
        // Everything else a session set, as it was given: temperature, top_p, stop.
        for (key, value) in request.settings.as_map().into_iter().flatten() {
            match key.as_str() {
                // The two this adapter reads above, in the fields Anthropic names.
                "reasoning_effort" | "max_tokens" => {}
                other => {
                    body.insert(other.to_string(), json_of(value));
                }
            }
        }
        // Anthropic keeps the system prompt out of the message list, so a `system` role is
        // lifted into a field and everything else becomes content blocks.
        if let Some(system) = system_prompt(&request.messages) {
            body.insert("system".into(), serde_json::Value::String(system));
        }
        body.insert("messages".into(), anthropic_messages(&request.messages));
        if let Some(tools) = tools_json(&request.tools) {
            let tools = tools
                .as_array()
                .map(|tools| {
                    serde_json::Value::Array(
                        tools
                            .iter()
                            .map(|tool| {
                                let name = tool.get("name").cloned().unwrap_or(serde_json::Value::Null);
                                let description = tool.get("description").cloned().unwrap_or(serde_json::Value::Null);
                                let schema = tool
                                    .get("input_schema")
                                    .and_then(|schema| serde_json::from_str::<serde_json::Value>(schema.as_str().unwrap_or("{}")).ok())
                                    .unwrap_or(serde_json::json!({"type": "object"}));
                                serde_json::json!({"name": name, "description": description, "input_schema": schema})
                            })
                            .collect(),
                    )
                })
                .unwrap_or(serde_json::Value::Null);
            body.insert("tools".into(), tools);
        }
        if let Value::Map(options) = &self.options {
            for (key, value) in options.iter() {
                body.insert(key.clone(), json_of(value));
            }
        }
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let mut request = Request::post(url, serde_json::Value::Object(body).to_string().into_bytes())
            .header("accept", "text/event-stream")
            .header("anthropic-version", "2023-06-01");
        if let Some(credential) = &self.credential {
            request = request.with_credential(credential.clone());
        }
        request
    }
}
#[async_trait]
impl Provider for AnthropicMessages {
    fn id(&self) -> &str {
        &self.id
    }

    fn api_base(&self) -> Option<&str> {
        Some(&self.base_url)
    }

    fn models_url(&self) -> Option<String> {
        Some(format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            self.models_path.trim_start_matches('/')
        ))
    }

    fn credential(&self) -> Option<Credential> {
        self.credential.clone()
    }

    async fn stream(
        &self,
        request: &ProviderRequest,
        id: &str,
        out: &mpsc::UnboundedSender<KernelEvent>,
    ) -> Result<Answer, String> {
        let mut answer = Answer::default();
        let mut calls = CallBuilder::default();
        let mut failure: Option<String> = None;
        // A tool call's index within the message, which is what a content block names.
        let mut position = 0usize;

        let result = self
            .http
            .stream(&self.request(request), |data| {
                let Ok(event) = serde_json::from_str::<serde_json::Value>(data) else {
                    return;
                };
                match event.get("type").and_then(|kind| kind.as_str()).unwrap_or("") {
                    "error" => failure = Some(event.to_string()),
                    "message_start" => {
                        if let Some(usage) = event.pointer("/message/usage") {
                            answer.input_tokens = usage.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        }
                    }
                    "content_block_start" => {
                        if let Some(block) = event.get("content_block")
                            && block.get("type").and_then(|kind| kind.as_str()) == Some("tool_use")
                        {
                            position = event.get("index").and_then(|index| index.as_u64()).unwrap_or(0) as usize;
                            calls.start(
                                position,
                                block.get("id").and_then(|id| id.as_str()),
                                block.get("name").and_then(|name| name.as_str()),
                            );
                        }
                    }
                    "content_block_delta" => {
                        let Some(delta) = event.get("delta") else {
                            return;
                        };
                        match delta.get("type").and_then(|kind| kind.as_str()).unwrap_or("") {
                            "text_delta" => {
                                if let Some(text) = delta.get("text").and_then(|text| text.as_str()) {
                                    answer.text.push_str(text);
                                    let _ =
                                        out.send(KernelEvent::ProviderDelta { id: id.to_string(), text: text.to_string() });
                                }
                            }
                            // Anthropic's thinking arrives as a block of its own, with the
                            // text in deltas called `thinking` rather than `text`. It is
                            // signed, and the signature is what lets it be replayed; this
                            // adapter does not replay it, so it does not keep it either.
                            "thinking_delta" => {
                                if let Some(text) = delta.get("thinking").and_then(|text| text.as_str()) {
                                    answer.thinking.push_str(text);
                                    let _ = out.send(KernelEvent::ProviderThinking {
                                        id: id.to_string(),
                                        text: text.to_string(),
                                    });
                                }
                            }
                            // Arguments arrive as a partial JSON string, one fragment at a
                            // time, which is why they are accumulated rather than parsed.
                            "input_json_delta" => {
                                if let Some(fragment) = delta.get("partial_json").and_then(|json| json.as_str()) {
                                    calls.args(position, fragment);
                                }
                            }
                            _ => {}
                        }
                    }
                    "message_delta" => {
                        if let Some(usage) = event.get("usage") {
                            answer.output_tokens = usage.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        }
                    }
                    _ => {}
                }
            })
            .await;

        if let Some(error) = failure {
            return Err(error);
        }
        result?;
        answer.tool_calls = calls.finish();
        Ok(answer)
    }
}

/// What an effort means when an effort is a budget of tokens.
///
/// Anthropic has no `reasoning_effort`; it has a budget, and the three levels are the shape a
/// person chose rather than a number they picked. The values are the ones Anthropic's own
/// documentation uses as examples, which is the only honest source for them.
fn thinking_budget(level: &str) -> i64 {
    match level {
        "low" => 2_048,
        "high" => 32_768,
        _ => 8_192,
    }
}

/// A [`Value`] as JSON, for the one place a provider wants a string of it.
///
/// A provider that takes a tool call wants its arguments as a JSON *string*, and
/// [`Value`]'s own `Display` is a compact reader's form (`{text: "hi"}`) rather than JSON.
/// Bytes have no JSON form, so they travel base64: a provider can be told what they are
/// but not handed them.
fn json_of(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(flag) => serde_json::Value::Bool(*flag),
        Value::Int(number) => serde_json::Value::Number((*number).into()),
        Value::Float(number) => serde_json::Number::from_f64(*number)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Str(text) => serde_json::Value::String(text.to_string()),
        Value::Bytes(bytes) => serde_json::Value::String(base64(bytes)),
        Value::List(items) => serde_json::Value::Array(items.iter().map(json_of).collect()),
        Value::Map(entries) => serde_json::Value::Object(
            entries.iter().map(|(key, value)| (key.clone(), json_of(value))).collect(),
        ),
    }
}

/// The tools, in the shape a provider expects.
///
/// A schema arrives as a string because that is what a declaration holds; a provider wants
/// an object, so it is parsed here rather than by every adapter.
fn tools_json(tools: &Value) -> Option<serde_json::Value> {
    let list = tools.as_list()?;
    if list.is_empty() {
        return None;
    }
    let out: Vec<serde_json::Value> = list
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?;
            let description = tool.get("description").and_then(Value::as_str).unwrap_or_default();
            let schema = tool
                .get("input_schema")
                .and_then(Value::as_str)
                .and_then(|schema| serde_json::from_str::<serde_json::Value>(schema).ok())
                .unwrap_or(serde_json::json!({"type": "object"}));
            Some(serde_json::json!({
                "type": "function",
                "function": {"name": name, "description": description, "parameters": schema},
            }))
        })
        .collect();
    Some(serde_json::Value::Array(out))
}
/// A session message's text, wherever the producer put it.
fn message_text(message: &Value) -> String {
    message
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| message.get("content").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

/// The attachments of a message, as `(media, bytes, missing)`.
///
/// The kernel resolved these before the request was handed over, so an attachment either
/// carries base64 bytes or says it could not be read. Nothing here fetches anything: a
/// provider adapter's job is a shape, not a capability.
fn attachment_parts(message: &Value) -> Vec<(String, String, bool)> {
    message
        .get("attachments")
        .and_then(Value::as_list)
        .map(|attachments| {
            attachments
                .iter()
                .map(|attachment| {
                    let missing = attachment.get("missing").and_then(Value::as_bool).unwrap_or(false);
                    (
                        attachment.get("media").and_then(Value::as_str).unwrap_or_default().to_string(),
                        attachment.get("data").and_then(Value::as_str).unwrap_or_default().to_string(),
                        missing,
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The tool calls of an assistant message, as `(id, name, args)`.
fn message_calls(message: &Value) -> Vec<(String, String, Value)> {
    message
        .get("tool_calls")
        .or_else(|| message.get("calls"))
        .and_then(Value::as_list)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let name = call.get("name").and_then(Value::as_str)?.to_string();
                    let id = call.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                    Some((id, name, call.get("args").cloned().unwrap_or(Value::Null)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The system prompt, joined, for a provider that keeps it out of the message list.
fn system_prompt(messages: &Value) -> Option<String> {
    let mut system: Vec<String> = Vec::new();
    for message in messages.as_list().unwrap_or(&[]) {
        if message.get("role").and_then(Value::as_str) == Some("system") {
            system.push(message_text(message));
        }
    }
    (!system.is_empty()).then(|| system.join("\n\n"))
}

/// A message as OpenAI wants it: `content` a string or an array of parts, tool calls with
/// their arguments as a JSON *string*, and a tool result that names the call it answers.
///
/// The shape is not cosmetic. A session's own message has `text` and `attachments` because
/// that is what it stores; a provider wants `content` and `image_url` parts, and a provider
/// that is sent the first instead of the second answers as if the message were empty.
/// The thinking a message carries, if it carries any.
fn message_thinking(message: &Value) -> String {
    message
        .get("thinking")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// One model a service says it has.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub id: String,
    pub label: String,
}

/// Ask a service which models it has.
///
/// The one request here that is not a completion, and the reason the session's catalog is
/// enrichment rather than the truth: a model list changes without a release, and a picker
/// that offers last month's models is a picker that lies. A service that answers with
/// something else — an error, an html page, a shape nobody documented — is a fault with its
/// body in it, not an empty list.
pub async fn discover(provider: &dyn Provider, http: &Http) -> Result<Vec<Listed>, String> {
    let Some(url) = provider.models_url() else {
        return Err(format!("`{}` does not list models", provider.id()));
    };
    let mut request = Request::get(url).header("accept", "application/json");
    if let Some(credential) = provider.credential() {
        request = request.with_credential(credential);
    }
    let response = http.send(&request).await?;
    let id = provider.id();
    if !response.ok() {
        return Err(format!(
            "{id} answered {} {}",
            response.status,
            response.text().chars().take(300).collect::<String>()
        ));
    }
    let body = response
        .json()
        .ok_or_else(|| format!("{id} answered with something that is not json"))?;
    Ok(parse_models(&body))
}

/// The models in a list response, however that service shaped it.
///
/// Three shapes are real: OpenAI's `{"data":[{"id":…}]}`, Anthropic's `{"data":[{"id":…,
/// "display_name":…}]}`, and the `{"models":[{"name":…}]}` a local server sometimes answers
/// with. Anything else has no models in it, which is a fact rather than a failure.
fn parse_models(body: &serde_json::Value) -> Vec<Listed> {
    let rows = body
        .get("data")
        .and_then(|rows| rows.as_array())
        .or_else(|| body.get("models").and_then(|rows| rows.as_array()));
    let Some(rows) = rows else {
        return Vec::new();
    };
    let mut models: Vec<Listed> = rows
        .iter()
        .filter_map(|row| {
            let id = row
                .get("id")
                .and_then(|id| id.as_str())
                .or_else(|| row.get("name").and_then(|name| name.as_str()))?;
            let label = row
                .get("display_name")
                .and_then(|label| label.as_str())
                .unwrap_or(id);
            Some(Listed { id: id.to_string(), label: label.to_string() })
        })
        .collect();
    // Sorted and deduplicated, because a picker wants a list and two spellings of one model is
    // a service being helpful rather than a second model.
    models.sort_by(|left, right| left.id.cmp(&right.id));
    models.dedup_by(|left, right| left.id == right.id);
    models
}

fn openai_messages_with(messages: &Value, dialect: Dialect) -> serde_json::Value {
    let mut out: Vec<serde_json::Value> = Vec::new();
    for message in messages.as_list().unwrap_or(&[]) {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("user");
        let text = message_text(message);
        match role {
            "assistant" => {
                let calls = message_calls(message);
                let mut entry = serde_json::Map::new();
                entry.insert("role".into(), serde_json::Value::String("assistant".into()));
                // OpenAI wants `null`, not `""`, for an assistant message that is nothing
                // but tool calls.
                entry.insert(
                    "content".into(),
                    if text.is_empty() && !calls.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::Value::String(text)
                    },
                );
                // A service that declared where captured thinking goes gets it back, and only
                // on a turn with tool calls it has to reason about: DeepSeek requires it there,
                // and every other service treats an unexpected field as a bad request.
                if let Some(field) = dialect.reasoning_field {
                    let thinking = message_thinking(message);
                    if !thinking.is_empty() || !calls.is_empty() {
                        entry.insert(field.into(), serde_json::Value::String(thinking));
                    }
                }
                if !calls.is_empty() {
                    entry.insert(
                        "tool_calls".into(),
                        serde_json::Value::Array(
                            calls
                                .into_iter()
                                .map(|(id, name, args)| {
                                    serde_json::json!({
                                        "id": id,
                                        "type": "function",
                                        "function": {"name": name, "arguments": json_of(&args).to_string()},
                                    })
                                })
                                .collect(),
                        ),
                    );
                }
                out.push(serde_json::Value::Object(entry));
            }
            "tool" => {
                out.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": message.get("call").and_then(Value::as_str).unwrap_or_default(),
                    "content": text,
                }));
            }
            _ => {
                let attachments = attachment_parts(message);
                let content = if attachments.is_empty() {
                    serde_json::Value::String(text)
                } else {
                    let mut parts: Vec<serde_json::Value> = Vec::new();
                    if !text.is_empty() {
                        parts.push(serde_json::json!({"type": "text", "text": text}));
                    }
                    for (media, data, missing) in attachments {
                        if missing {
                            parts.push(serde_json::json!({
                                "type": "text",
                                "text": "[attachment unavailable: its bytes are not in this daemon's store]",
                            }));
                            continue;
                        }
                        parts.push(serde_json::json!({
                            "type": "image_url",
                            "image_url": {"url": format!("data:{media};base64,{data}")},
                        }));
                    }
                    serde_json::Value::Array(parts)
                };
                out.push(serde_json::json!({"role": role, "content": content}));
            }
        }
    }
    serde_json::Value::Array(out)
}

/// A message as Anthropic wants it: content blocks, `tool_use` for a call, and a
/// `tool_result` block inside a user message for its answer.
fn anthropic_messages(messages: &Value) -> serde_json::Value {
    let mut out: Vec<serde_json::Value> = Vec::new();
    for message in messages.as_list().unwrap_or(&[]) {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("user");
        if role == "system" {
            // Lifted into the request's own field, which is where Anthropic reads it.
            continue;
        }
        let mut blocks: Vec<serde_json::Value> = Vec::new();
        let text = message_text(message);
        match role {
            "assistant" => {
                if !text.is_empty() {
                    blocks.push(serde_json::json!({"type": "text", "text": text}));
                }
                for (id, name, args) in message_calls(message) {
                    let input = json_of(&args);
                    blocks.push(serde_json::json!({"type": "tool_use", "id": id, "name": name, "input": input}));
                }
                if blocks.is_empty() {
                    continue;
                }
                out.push(serde_json::json!({"role": "assistant", "content": blocks}));
            }
            "tool" => {
                // A tool's answer is a `tool_result` block, and it has to travel inside a
                // user message because that is the only place Anthropic reads one.
                out.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": message.get("call").and_then(Value::as_str).unwrap_or_default(),
                        "content": text,
                    }],
                }));
            }
            _ => {
                if !text.is_empty() {
                    blocks.push(serde_json::json!({"type": "text", "text": text}));
                }
                for (media, data, missing) in attachment_parts(message) {
                    if missing {
                        blocks.push(serde_json::json!({
                            "type": "text",
                            "text": "[attachment unavailable: its bytes are not in this daemon's store]",
                        }));
                        continue;
                    }
                    blocks.push(serde_json::json!({
                        "type": "image",
                        "source": {"type": "base64", "media_type": media, "data": data},
                    }));
                }
                if blocks.is_empty() {
                    continue;
                }
                out.push(serde_json::json!({"role": role, "content": blocks}));
            }
        }
    }
    serde_json::Value::Array(out)
}

/// OpenAI's responses API, which is a *different shape* from chat completions.
///
/// It is here rather than in [`OpenAiChat`] because it is a different protocol:
/// `input` items instead of `messages`, `function_call`/`function_call_output`
/// items instead of a tool-call list, `max_output_tokens` instead of a token
/// budget, and a stream whose events name the item they belong to rather than
/// indexing into a choice. Treating it as chat completions with a flag is how a
/// frontend ends up with an answer that never arrives.
///
/// The one thing it shares with the chat adapter is that everything above it is
/// the same: a session hands over its own messages and settings, and this turns
/// them into what this service wants.
pub struct OpenAiResponses {
    id: String,
    base_url: String,
    models_path: &'static str,
    credential: Option<Credential>,
    http: Arc<Http>,
    options: Value,
    effort: Effort,
}

impl OpenAiResponses {
    pub fn new(id: impl Into<String>, base_url: impl Into<String>, http: Arc<Http>) -> OpenAiResponses {
        OpenAiResponses {
            id: id.into(),
            base_url: base_url.into(),
            models_path: "models",
            credential: None,
            http,
            options: Value::Null,
            effort: Effort::Reasoning,
        }
    }

    /// An adapter for one of the services this daemon knows by name.
    pub fn from_preset(preset: &Preset, http: Arc<Http>) -> OpenAiResponses {
        let responses = OpenAiResponses::new(preset.id, preset.base_url, http)
            .with_models_path(preset.models_path)
            .with_effort(preset.effort);
        match preset.credential() {
            Some(credential) => responses.with_credential(credential),
            None => responses,
        }
    }

    pub fn with_models_path(mut self, models_path: &'static str) -> OpenAiResponses {
        self.models_path = models_path;
        self
    }

    pub fn with_effort(mut self, effort: Effort) -> OpenAiResponses {
        self.effort = effort;
        self
    }

    pub fn credentialed(mut self, slot: impl Into<String>) -> OpenAiResponses {
        self.credential = Some(Credential::bearer(slot));
        self
    }

    pub fn with_credential(mut self, credential: Credential) -> OpenAiResponses {
        self.credential = Some(credential);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> OpenAiResponses {
        self.base_url = base_url.into();
        self
    }

    pub fn with_options(mut self, options: Value) -> OpenAiResponses {
        self.options = options;
        self
    }

    fn request(&self, request: &ProviderRequest) -> Request {
        let mut body = serde_json::Map::new();
        body.insert("model".into(), serde_json::Value::String(request.model.clone()));
        body.insert("stream".into(), serde_json::Value::Bool(true));
        // The backend is told not to keep the conversation: this daemon's log is
        // the truth, and a service that stored a second copy could not be asked to
        // forget a branch.
        body.insert("store".into(), serde_json::Value::Bool(false));
        if let Some(instructions) = system_prompt(&request.messages) {
            body.insert("instructions".into(), serde_json::Value::String(instructions));
        }
        body.insert("input".into(), responses_input(&request.messages));
        if let Some(tools) = responses_tools(&request.tools) {
            body.insert("tools".into(), tools);
        }
        for (key, value) in request.settings.as_map().into_iter().flatten() {
            match key.as_str() {
                "reasoning_effort" => {
                    if let (Effort::Reasoning, Some(level)) = (self.effort, value.as_str()) {
                        body.insert("reasoning".into(), serde_json::json!({"effort": level}));
                    }
                }
                // The session's `max_tokens` is this service's output budget, and
                // its own name for it.
                "max_tokens" => {
                    body.insert("max_output_tokens".into(), json_of(value));
                }
                other => {
                    body.insert(other.to_string(), json_of(value));
                }
            }
        }
        if let Value::Map(options) = &self.options {
            for (key, value) in options.iter() {
                body.insert(key.clone(), json_of(value));
            }
        }
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let mut request = Request::post(url, serde_json::Value::Object(body).to_string().into_bytes())
            .header("accept", "text/event-stream");
        if let Some(credential) = &self.credential {
            request = request.with_credential(credential.clone());
        }
        request
    }
}

#[async_trait]
impl Provider for OpenAiResponses {
    fn id(&self) -> &str {
        &self.id
    }

    fn models_url(&self) -> Option<String> {
        Some(format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            self.models_path.trim_start_matches('/')
        ))
    }

    fn credential(&self) -> Option<Credential> {
        self.credential.clone()
    }

    async fn stream(
        &self,
        request: &ProviderRequest,
        id: &str,
        out: &mpsc::UnboundedSender<KernelEvent>,
    ) -> Result<Answer, String> {
        let mut answer = Answer::default();
        let mut failure: Option<String> = None;
        // Indexed by the item's position in the response, because that is what the
        // stream's own events name. A call assembled from a delta and then
        // completed in full is *replaced*, not appended to, or its arguments would
        // be the fragment twice.
        let mut calls: std::collections::BTreeMap<usize, PartialCall> = std::collections::BTreeMap::new();

        let result = self
            .http
            .stream(&self.request(request), |data| {
                let Ok(event) = serde_json::from_str::<serde_json::Value>(data) else {
                    return;
                };
                let kind = event.get("type").and_then(|kind| kind.as_str()).unwrap_or_default();
                match kind {
                    "response.output_text.delta" => {
                        if let Some(text) = event.get("delta").and_then(|delta| delta.as_str()) {
                            answer.text.push_str(text);
                            let _ =
                                out.send(KernelEvent::ProviderDelta { id: id.to_string(), text: text.to_string() });
                        }
                    }
                    // Reasoning arrives under two names: OpenAI streams a summary,
                    // and services that imitate it stream the reasoning itself.
                    "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                        if let Some(text) = event.get("delta").and_then(|delta| delta.as_str()) {
                            answer.thinking.push_str(text);
                            let _ =
                                out.send(KernelEvent::ProviderThinking { id: id.to_string(), text: text.to_string() });
                        }
                    }
                    "response.output_item.added" | "response.output_item.done" => {
                        let Some(item) = event.get("item") else {
                            return;
                        };
                        if item.get("type").and_then(|item| item.as_str()) != Some("function_call") {
                            return;
                        }
                        let index = event.get("output_index").and_then(|index| index.as_u64()).unwrap_or(0) as usize;
                        let call = calls.entry(index).or_default();
                        if let Some(call_id) = item.get("call_id").and_then(|call| call.as_str()) {
                            call.id = call_id.to_string();
                        }
                        if let Some(name) = item.get("name").and_then(|name| name.as_str()) {
                            call.name = name.to_string();
                        }
                        // A finished item carries the whole argument string, which is
                        // the truth and replaces whatever fragments arrived.
                        if kind == "response.output_item.done"
                            && let Some(arguments) = item.get("arguments").and_then(|args| args.as_str())
                        {
                            call.args = arguments.to_string();
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        let index = event.get("output_index").and_then(|index| index.as_u64()).unwrap_or(0) as usize;
                        if let Some(delta) = event.get("delta").and_then(|delta| delta.as_str()) {
                            calls.entry(index).or_default().args.push_str(delta);
                        }
                    }
                    "response.completed" => {
                        if let Some(response) = event.get("response") {
                            if let Some(usage) = response.get("usage") {
                                answer.input_tokens =
                                    usage.get("input_tokens").and_then(|value| value.as_i64()).unwrap_or(0);
                                answer.output_tokens =
                                    usage.get("output_tokens").and_then(|value| value.as_i64()).unwrap_or(0);
                            }
                            // The completed response is the authoritative list: it
                            // carries every call in full, so whatever streamed before
                            // it is superseded. Replacing rather than merging is what
                            // keeps one call from arriving twice, once as fragments
                            // and once whole.
                            if let Some(output) = response.get("output").and_then(|output| output.as_array()) {
                                let mut complete: std::collections::BTreeMap<usize, PartialCall> =
                                    std::collections::BTreeMap::new();
                                for (position, item) in output.iter().enumerate() {
                                    if item.get("type").and_then(|item| item.as_str()) != Some("function_call") {
                                        continue;
                                    }
                                    let index = item
                                        .get("output_index")
                                        .and_then(|index| index.as_u64())
                                        .map(|index| index as usize)
                                        .unwrap_or(position);
                                    let call = complete.entry(index).or_default();
                                    if let Some(name) = item.get("name").and_then(|name| name.as_str()) {
                                        call.name = name.to_string();
                                    }
                                    if let Some(call_id) = item.get("call_id").and_then(|call| call.as_str()) {
                                        call.id = call_id.to_string();
                                    }
                                    if let Some(arguments) = item.get("arguments").and_then(|args| args.as_str()) {
                                        call.args = arguments.to_string();
                                    }
                                }
                                if !complete.is_empty() {
                                    calls = complete;
                                }
                            }
                        }
                    }
                    "response.failed" | "response.incomplete" | "error" => {
                        let message = event
                            .get("response")
                            .and_then(|response| response.get("error"))
                            .and_then(|error| error.get("message"))
                            .and_then(|message| message.as_str())
                            .or_else(|| event.get("message").and_then(|message| message.as_str()))
                            .or_else(|| event.get("error").and_then(|error| error.as_str()))
                            .unwrap_or("the response did not complete");
                        failure = Some(message.to_string());
                    }
                    _ => {}
                }
            })
            .await;

        if let Some(message) = failure {
            return Err(message);
        }
        result?;
        answer.tool_calls = Value::list(
            calls
                .into_values()
                .filter(PartialCall::usable)
                .enumerate()
                .map(|(index, mut call)| {
                    if call.id.is_empty() {
                        call.id = format!("call.{index}");
                    }
                    call.to_value()
                })
                .collect::<Vec<_>>(),
        );
        Ok(answer)
    }
}

/// The session's messages, as responses-api input items.
///
/// Three item kinds, and the distinction is the whole protocol: a message is a
/// list of content parts, a tool call is an item of its own rather than a field
/// on the assistant message, and a tool's answer names the call it answers by
/// `call_id` rather than by a role.
fn responses_input(messages: &Value) -> serde_json::Value {
    let mut out: Vec<serde_json::Value> = Vec::new();
    for message in messages.as_list().unwrap_or(&[]) {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("user");
        match role {
            // The system prompt is `instructions`, which the request builder lifts
            // out; a system message left here would be an item this api rejects.
            "system" => continue,
            "tool" => {
                out.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": message
                        .get("tool_call_id")
                        .or_else(|| message.get("call_id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    "output": message_text(message),
                }));
            }
            "assistant" => {
                let text = message_text(message);
                if !text.is_empty() {
                    out.push(serde_json::json!({
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": text}],
                    }));
                }
                for (call_id, name, args) in message_calls(message) {
                    out.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": call_id,
                        "name": name,
                        "arguments": json_of(&args).to_string(),
                    }));
                }
            }
            _ => {
                let mut content = vec![serde_json::json!({"type": "input_text", "text": message_text(message)})];
                for (media, data, missing) in attachment_parts(message) {
                    // An attachment nobody can read is said to be there, because a
                    // model told nothing answers as if nothing was attached.
                    if missing {
                        content.push(serde_json::json!({
                            "type": "input_text",
                            "text": "[an attachment was here and could not be read]",
                        }));
                    } else {
                        content.push(serde_json::json!({
                            "type": "input_image",
                            "image_url": format!("data:{media};base64,{data}"),
                        }));
                    }
                }
                out.push(serde_json::json!({"role": role, "content": content}));
            }
        }
    }
    serde_json::Value::Array(out)
}

/// The tools, in the flat shape this api wants: the name is the function's, not
/// a field of a nested object.
fn responses_tools(tools: &Value) -> Option<serde_json::Value> {
    let list = tools.as_list()?;
    if list.is_empty() {
        return None;
    }
    let out: Vec<serde_json::Value> = list
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?;
            let description = tool.get("description").and_then(Value::as_str).unwrap_or_default();
            let parameters = tool
                .get("input_schema")
                .and_then(Value::as_str)
                .and_then(|schema| serde_json::from_str::<serde_json::Value>(schema).ok())
                .unwrap_or(serde_json::json!({"type": "object"}));
            Some(serde_json::json!({
                "type": "function",
                "name": name,
                "description": description,
                "parameters": parameters,
                "strict": false,
            }))
        })
        .collect();
    Some(serde_json::Value::Array(out))
}

/// A provider that plays a script, for tests and for a session with no network.
///
/// Not a stub: a scripted provider is what makes the agent loop testable without an
/// account, without a network, and without spending anything, which the previous system
/// also decided it wanted and got a great deal of value from.
pub struct ScriptedProvider {
    turns: std::sync::Mutex<std::collections::VecDeque<Turn>>,
    id: String,
    fallback: String,
}

impl ScriptedProvider {
    pub fn new(turns: impl IntoIterator<Item = Turn>) -> Arc<ScriptedProvider> {
        Arc::new(ScriptedProvider {
            turns: std::sync::Mutex::new(turns.into_iter().collect()),
            id: "scripted".into(),
            fallback: "…".into(),
        })
    }

    /// A provider that always answers the same way.
    pub fn always(text: impl Into<String>) -> Arc<ScriptedProvider> {
        ScriptedProvider::new([Turn::say(text)])
    }

    pub fn named(mut self, id: impl Into<String>) -> ScriptedProvider {
        self.id = id.into();
        self
    }

    pub fn remaining(&self) -> usize {
        self.turns.lock().expect("script is never poisoned").len()
    }

    fn next_turn(&self) -> Option<Turn> {
        self.turns.lock().expect("script is never poisoned").pop_front()
    }
}

/// One scripted thing a provider does when asked.
#[derive(Clone, Debug)]
pub enum Turn {
    Say { text: String, chunk: usize },
    Call { name: String, args: Value, then: Box<Turn> },
}

impl Turn {
    pub fn say(text: impl Into<String>) -> Turn {
        Turn::Say { text: text.into(), chunk: 24 }
    }

    pub fn call(name: impl Into<String>, args: Value, then: Turn) -> Turn {
        Turn::Call { name: name.into(), args, then: Box::new(then) }
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn stream(
        &self,
        _request: &ProviderRequest,
        id: &str,
        out: &mpsc::UnboundedSender<KernelEvent>,
    ) -> Result<Answer, String> {
        let mut answer = Answer::default();
        let turn = self.next_turn().unwrap_or_else(|| Turn::say(self.fallback.clone()));
        let (text, tool_calls) = match turn {
            Turn::Say { text, chunk } => {
                let chunk = chunk.max(1);
                let characters: Vec<char> = text.chars().collect();
                let mut emitted = 0usize;
                while emitted < characters.len() {
                    let end = (emitted + chunk).min(characters.len());
                    let piece: String = characters[emitted..end].iter().collect();
                    let _ = out.send(KernelEvent::ProviderDelta { id: id.to_string(), text: piece });
                    tokio::task::yield_now().await;
                    emitted = end;
                }
                answer.input_tokens = 128;
                answer.output_tokens = characters.len() as i64 / 4;
                (text, Value::list([]))
            }
            Turn::Call { name, args, then } => {
                let call_id = format!("{id}.call.1");
                let implicit = match *then {
                    Turn::Say { text, .. } => text,
                    other => {
                        self.turns.lock().expect("script is never poisoned").push_front(other);
                        String::new()
                    }
                };
                answer.input_tokens = 128;
                (
                    implicit,
                    Value::list([Value::map([
                        ("id", Value::str(&call_id)),
                        ("name", Value::str(&name)),
                        ("args", args),
                    ])]),
                )
            }
        };
        answer.text = text;
        answer.tool_calls = tool_calls;
        Ok(answer)
    }
}

/// A provider that always fails, for the failure path.
pub struct BrokenProvider {
    id: String,
    message: String,
}

impl BrokenProvider {
    pub fn new(id: impl Into<String>, message: impl Into<String>) -> Arc<BrokenProvider> {
        Arc::new(BrokenProvider { id: id.into(), message: message.into() })
    }
}

#[async_trait]
impl Provider for BrokenProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn stream(
        &self,
        _request: &ProviderRequest,
        _id: &str,
        _out: &mpsc::UnboundedSender<KernelEvent>,
    ) -> Result<Answer, String> {
        Err(self.message.clone())
    }
}

/// Whether a base url and a credential look usable, for a diagnostic before a call.
pub fn check_ready(base_url: &str, credential: Option<&str>, credentials: &Credentials) -> Result<(), String> {
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return Err(format!("`{base_url}` is not an http url"));
    }
    if let Some(slot) = credential
        && !credentials.has(slot)
    {
        return Err(format!("there is no credential for `{slot}`; add one with the login command"));
    }
    Ok(())
}

/// A hint for a mistyped credential, used by the login command.
pub fn credential_looks_wrong(value: &str) -> bool {
    !looks_like_a_secret(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default dialect, for the tests that are about the shape of a message rather than
    /// about what one service calls a field.
    fn openai_messages(messages: &Value) -> serde_json::Value {
        openai_messages_with(messages, Dialect::default())
    }
    use crate::credentials::Credentials;

    fn http() -> Arc<Http> {
        Arc::new(Http::new(Arc::new(Credentials::in_memory())).unwrap())
    }

    /// A server that answers one request with a canned server-sent event stream.
    ///
    /// Hand-written rather than a framework: what is being tested is what the *wire*
    /// does, and a framework that agreed with my assumptions would hide the difference.
    struct SseServer {
        address: String,
        handle: tokio::task::JoinHandle<Vec<String>>,
    }

    impl SseServer {
        async fn start(frames: Vec<String>, status: &'static str) -> SseServer {
            SseServer::start_with(frames, status, "text/event-stream").await
        }

        /// The same server, answering with whatever a test is about: a model list is json, and
        /// a refusal is json too.
        async fn start_with(frames: Vec<String>, status: &'static str, content_type: &'static str) -> SseServer {
            let content_type = content_type.to_string();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap().to_string();
            let handle = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
                let mut request = vec![0u8; 16 * 1024];
                let mut seen = 0usize;
                // Read the headers, then the body if there is one.
                loop {
                    let read = stream.read(&mut request[seen..]).await.unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    seen += read;
                    let text = String::from_utf8_lossy(&request[..seen]).to_string();
                    if let Some(index) = text.find("\r\n\r\n") {
                        let length = text
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if seen >= index + 4 + length {
                            break;
                        }
                    }
                }
                let body = frames.join("");
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
                vec![String::from_utf8_lossy(&request[..seen]).to_string()]
            });
            SseServer { address, handle }
        }

        async fn requests(self) -> Vec<String> {
            self.handle.await.unwrap()
        }

        /// One json answer, which is what a model list is.
        async fn json(body: &str, status: &'static str) -> SseServer {
            SseServer::start_with(vec![body.to_string()], status, "application/json").await
        }
    }

    fn event(json: &str) -> String {
        format!("data: {json}\n\n")
    }

    #[tokio::test]
    async fn an_openai_stream_becomes_deltas_and_a_whole_answer() {
        let server = SseServer::start(
            vec![
                event(r#"{"choices":[{"delta":{"content":"Hello"}}]}"#),
                event(r#"{"choices":[{"delta":{"content":", world"}}]}"#),
                event(r#"{"usage":{"prompt_tokens":11,"completion_tokens":3}}"#),
                "data: [DONE]\n\n".to_string(),
            ],
            "200 OK",
        )
        .await;
        let provider = OpenAiChat::new("openai", format!("http://{}", server.address), http());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let answer = provider
            .stream(
                &ProviderRequest {
                    model: "gpt".into(),
                    messages: Value::list([Value::map([("role", Value::str("user")), ("text", Value::str("hi"))])]),
                    tools: Value::list([]),
                    settings: Value::map([]),
                },
                "r1",
                &tx,
            )
            .await
            .expect("a stream");
        assert_eq!(answer.text, "Hello, world");
        assert_eq!(answer.input_tokens, 11);
        assert_eq!(answer.output_tokens, 3);
        let mut deltas = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let KernelEvent::ProviderDelta { text, .. } = event {
                deltas.push(text);
            }
        }
        assert_eq!(deltas, vec!["Hello".to_string(), ", world".to_string()]);
        let requests = server.requests().await;
        assert!(requests[0].contains("POST /chat/completions"), "{}", requests[0]);
        assert!(requests[0].contains("text/event-stream"), "{}", requests[0]);
        assert!(requests[0].contains("\"stream\":true"), "{}", requests[0]);
    }

    #[tokio::test]
    async fn openai_tool_calls_are_assembled_from_pieces() {
        let server = SseServer::start(
            vec![
                event(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file"}}]}}]}"#),
                event(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}"#),
                event(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"src/main.rs\"}"}}]}}]}"#),
                "data: [DONE]\n\n".to_string(),
            ],
            "200 OK",
        )
        .await;
        let provider = OpenAiChat::new("openai", format!("http://{}", server.address), http());
        let (tx, _rx) = mpsc::unbounded_channel();
        let answer = provider
            .stream(
                &ProviderRequest { model: "gpt".into(), messages: Value::list([]), tools: Value::list([]), settings: Value::map([]) },
                "r1",
                &tx,
            )
            .await
            .unwrap();
        let calls = answer.tool_calls.as_list().expect("calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].get("name").and_then(Value::as_str), Some("read_file"));
        assert_eq!(calls[0].get("id").and_then(Value::as_str), Some("call_1"));
        // The arguments were a JSON string in three fragments and are an object now.
        assert_eq!(
            calls[0].get("args").and_then(|args| args.get("path")).and_then(Value::as_str),
            Some("src/main.rs")
        );
    }

    #[tokio::test]
    async fn an_anthropic_stream_lifts_the_system_prompt_and_reads_typed_deltas() {
        let server = SseServer::start(
            vec![
                event(r#"{"type":"message_start","message":{"usage":{"input_tokens":42}}}"#),
                event(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"shell"}}"#),
                event(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}"#),
                event(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"echo hi\"}"}}"#),
                event(r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"done"}}"#),
                event(r#"{"type":"message_delta","usage":{"output_tokens":7}}"#),
            ],
            "200 OK",
        )
        .await;
        let provider = AnthropicMessages::new("anthropic", format!("http://{}", server.address), http());
        let (tx, _rx) = mpsc::unbounded_channel();
        let answer = provider
            .stream(
                &ProviderRequest {
                    model: "claude".into(),
                    messages: Value::list([
                        Value::map([("role", Value::str("system")), ("text", Value::str("be brief"))]),
                        Value::map([("role", Value::str("user")), ("text", Value::str("hi"))]),
                    ]),
                    tools: Value::list([]),
                    settings: Value::map([]),
                },
                "r1",
                &tx,
            )
            .await
            .unwrap();
        assert_eq!(answer.text, "done");
        assert_eq!(answer.input_tokens, 42);
        assert_eq!(answer.output_tokens, 7);
        let calls = answer.tool_calls.as_list().expect("calls");
        assert_eq!(calls[0].get("name").and_then(Value::as_str), Some("shell"));
        assert_eq!(
            calls[0].get("args").and_then(|args| args.get("command")).and_then(Value::as_str),
            Some("echo hi")
        );
        let requests = server.requests().await;
        // The system prompt is a field, not a message, and the version header is there.
        assert!(requests[0].contains("\"system\":\"be brief\""), "{}", requests[0]);
        assert!(requests[0].contains("anthropic-version"), "{}", requests[0]);
    }

    #[tokio::test]
    async fn each_service_gets_the_word_it_uses_for_a_token_budget() {
        // The field is the service's, not the session's: OpenAI's reasoning models accept only
        // `max_completion_tokens`, and everything else accepts `max_tokens` and rejects the
        // other one. A session sets a budget; an adapter knows what to call it.
        let server = SseServer::json("{}", "200 OK").await;
        let chat = OpenAiChat::new("openai", format!("http://{}", server.address), http())
            .with_dialect(crate::presets::preset("openai").expect("a preset").into());
        let (tx, _rx) = mpsc::unbounded_channel();
        let _ = chat
            .stream(
                &ProviderRequest {
                    model: "gpt".into(),
                    messages: Value::list([]),
                    tools: Value::list([]),
                    settings: Value::map([("max_tokens", Value::Int(1_000))]),
                },
                "r1",
                &tx,
            )
            .await;
        let requests = server.requests().await;
        assert!(requests[0].contains(r#""max_completion_tokens":1000"#), "{}", requests[0]);
        assert!(!requests[0].contains(r#""max_tokens"#), "{}", requests[0]);

        // And the same adapter for a service that wants the older name.
        let server = SseServer::json("{}", "200 OK").await;
        let chat = OpenAiChat::new("deepseek", format!("http://{}", server.address), http())
            .with_dialect(crate::presets::preset("deepseek").expect("a preset").into());
        let _ = chat
            .stream(
                &ProviderRequest {
                    model: "deepseek-chat".into(),
                    messages: Value::list([]),
                    tools: Value::list([]),
                    settings: Value::map([("max_tokens", Value::Int(1_000))]),
                },
                "r1",
                &tx,
            )
            .await;
        let requests = server.requests().await;
        assert!(requests[0].contains(r#""max_tokens":1000"#), "{}", requests[0]);
    }

    #[tokio::test]
    async fn an_effort_is_spelled_the_way_the_service_spells_it() {
        let attempt = |id: &'static str| async move {
            let server = SseServer::json("{}", "200 OK").await;
            let chat = OpenAiChat::new(id, format!("http://{}", server.address), http())
                .with_dialect(crate::presets::preset(id).expect("a preset").into());
            let (tx, _rx) = mpsc::unbounded_channel();
            let _ = chat
                .stream(
                    &ProviderRequest {
                        model: "m".into(),
                        messages: Value::list([]),
                        tools: Value::list([]),
                        settings: Value::map([("reasoning_effort", Value::str("high"))]),
                    },
                    "r1",
                    &tx,
                )
                .await;
            server.requests().await.remove(0)
        };
        // OpenAI and the services that copied it.
        let openai = attempt("openai").await;
        assert!(openai.contains(r#""reasoning_effort":"high""#), "{openai}");
        // OpenRouter, which proxies many services and spells it its own way.
        let openrouter = attempt("openrouter").await;
        assert!(openrouter.contains(r#""reasoning":{"effort":"high"}"#), "{openrouter}");
        assert!(!openrouter.contains("reasoning_effort"), "{openrouter}");
        // A service with no such setting is not sent one: an unknown field is a 400.
        let cerebras = attempt("cerebras").await;
        assert!(!cerebras.contains("reasoning"), "{cerebras}");
    }

    #[tokio::test]
    async fn a_service_that_reasons_aloud_has_its_reasoning_kept_apart_from_the_answer() {
        // The deltas are what a service sends; the answer is what a person asked for. Both
        // arrive, on channels that say which is which.
        let server = SseServer::start(
            vec![
                event(r#"{"choices":[{"delta":{"reasoning_content":"let me see"}}]}"#),
                event(r#"{"choices":[{"delta":{"reasoning_content":" — no, this"}}]}"#),
                event(r#"{"choices":[{"delta":{"content":"the answer"}}]}"#),
                "data: [DONE]\n\n".to_string(),
            ],
            "200 OK",
        )
        .await;
        let provider = OpenAiChat::new("deepseek", format!("http://{}", server.address), http());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let answer = provider
            .stream(
                &ProviderRequest {
                    model: "deepseek-reasoner".into(),
                    messages: Value::list([]),
                    tools: Value::list([]),
                    settings: Value::map([]),
                },
                "r1",
                &tx,
            )
            .await
            .expect("a stream");
        assert_eq!(answer.text, "the answer");
        assert_eq!(answer.thinking, "let me see — no, this");
        let mut thinking = Vec::new();
        let mut text = Vec::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                KernelEvent::ProviderThinking { text: piece, .. } => thinking.push(piece),
                KernelEvent::ProviderDelta { text: piece, .. } => text.push(piece),
                _ => {}
            }
        }
        assert_eq!(thinking, vec!["let me see".to_string(), " — no, this".to_string()]);
        assert_eq!(text, vec!["the answer".to_string()]);
    }

    #[tokio::test]
    async fn reasoning_goes_back_only_to_the_service_that_needs_it() {
        // DeepSeek rejects a tool-call turn whose reasoning was not sent back with it, and
        // every other service treats an unexpected field as a bad request. So the field is
        // sent when the service declared it, and not otherwise.
        let messages = Value::list([Value::map([
            ("role", Value::str("assistant")),
            ("text", Value::str("")),
            ("thinking", Value::str("I should call echo")),
            (
                "calls",
                Value::list([Value::map([
                    ("id", Value::str("c1")),
                    ("name", Value::str("echo")),
                    ("args", Value::map([("text", Value::str("hi"))])),
                ])]),
            ),
        ])]);
        let deepseek = openai_messages_with(
            &messages,
            Dialect { reasoning_field: Some("reasoning_content"), ..Dialect::default() },
        );
        assert_eq!(deepseek[0]["reasoning_content"], "I should call echo");
        let openai = openai_messages(&messages);
        assert!(openai[0].get("reasoning_content").is_none(), "{}", openai[0]);
    }

    #[tokio::test]
    async fn an_anthropic_effort_is_a_thinking_budget_inside_the_answer_that_follows_it() {
        let server = SseServer::start(
            vec![
                event(r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}"#),
                event(r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"done"}}"#),
            ],
            "200 OK",
        )
        .await;
        let provider = AnthropicMessages::new("anthropic", format!("http://{}", server.address), http());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let answer = provider
            .stream(
                &ProviderRequest {
                    model: "claude".into(),
                    messages: Value::list([]),
                    tools: Value::list([]),
                    settings: Value::map([("reasoning_effort", Value::str("high"))]),
                },
                "r1",
                &tx,
            )
            .await
            .expect("a stream");
        assert_eq!(answer.text, "done");
        assert_eq!(answer.thinking, "hmm");
        let mut thought = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, KernelEvent::ProviderThinking { .. }) {
                thought = true;
            }
        }
        assert!(thought, "thinking was not reported as thinking");

        let requests = server.requests().await;
        let body = &requests[0];
        // A budget to think with, and room for the answer after it: a thinking budget that
        // filled the whole allowance would be a request that always ends in the middle.
        assert!(body.contains(r#""thinking":{"budget_tokens":32768,"type":"enabled"}"#), "{body}");
        assert!(body.contains(r#""max_tokens":40960"#), "{body}");
        // And an effort is not sent as a word to a service that has no such word.
        assert!(!body.contains("reasoning_effort"), "{body}");
    }

    #[tokio::test]
    async fn a_model_list_is_read_out_of_whatever_shape_it_arrived_in() {
        // OpenAI's shape.
        let server = SseServer::json(
            r#"{"object":"list","data":[{"id":"m-2","created":1},{"id":"m-1"}]}"#,
            "200 OK",
        )
        .await;
        let provider = OpenAiChat::new("openai", format!("http://{}", server.address), http());
        let models = discover(&provider, &http()).await.expect("a list");
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), vec!["m-1", "m-2"]);
        assert_eq!(models[1].label, "m-2");
        let requests = server.requests().await;
        assert!(requests[0].contains("GET /models"), "{}", requests[0]);

        // Anthropic's, which names a display label as well as an id, and lives at
        // `/v1/models` rather than `/models`.
        let server = SseServer::json(r#"{"data":[{"id":"claude-x","display_name":"Claude X"}]}"#, "200 OK").await;
        let provider = AnthropicMessages::new("anthropic", format!("http://{}", server.address), http());
        let models = discover(&provider, &http()).await.expect("a list");
        assert_eq!(models[0].label, "Claude X");
        let requests = server.requests().await;
        assert!(requests[0].contains("GET /v1/models"), "{}", requests[0]);

        // A local server's, which is neither.
        let server = SseServer::json(r#"{"models":[{"name":"llama-3"}]}"#, "200 OK").await;
        let provider = OpenAiChat::new("ollama", format!("http://{}", server.address), http());
        let models = discover(&provider, &http()).await.expect("a list");
        assert_eq!(models[0].id, "llama-3");

        // A script has no models to list, and says so rather than answering with nothing.
        let error = discover(&*ScriptedProvider::always("hi"), &http()).await.unwrap_err();
        assert!(error.contains("does not list models"), "{error}");
    }

    #[tokio::test]
    async fn a_model_list_that_is_really_a_refusal_is_reported_as_one() {
        let server = SseServer::json(r#"{"error":{"message":"invalid api key"}}"#, "401 Unauthorized").await;
        let provider = OpenAiChat::new("openai", format!("http://{}", server.address), http());
        let error = discover(&provider, &http()).await.unwrap_err();
        assert!(error.contains("401"), "{error}");
        assert!(error.contains("invalid api key"), "{error}");

        // And a page that is not a model list is not a model list.
        let server = SseServer::json("<html>not this</html>", "200 OK").await;
        let provider = OpenAiChat::new("openai", format!("http://{}", server.address), http());
        let error = discover(&provider, &http()).await.unwrap_err();
        assert!(error.contains("not json"), "{error}");
    }

    #[tokio::test]
    async fn a_refusal_is_an_error_with_the_body_in_it() {
        let server = SseServer::start(
            vec![r#"{"error":{"type":"invalid_request_error","message":"no such model"}}"#.to_string()],
            "400 Bad Request",
        )
        .await;
        let provider = OpenAiChat::new("openai", format!("http://{}", server.address), http());
        let (tx, _rx) = mpsc::unbounded_channel();
        let error = provider
            .stream(
                &ProviderRequest { model: "nope".into(), messages: Value::list([]), tools: Value::list([]), settings: Value::map([]) },
                "r1",
                &tx,
            )
            .await
            .unwrap_err();
        assert!(error.contains("no such model"), "{error}");
    }

    #[tokio::test]
    async fn a_missing_credential_is_refused_before_the_request_goes_out() {
        // Nothing listens on this port; if the credential check did not happen first,
        // this would be a connection error instead.
        let provider = OpenAiChat::new("openai", "http://127.0.0.1:1", http()).credentialed("nobody");
        let (tx, _rx) = mpsc::unbounded_channel();
        let error = provider
            .stream(
                &ProviderRequest { model: "m".into(), messages: Value::list([]), tools: Value::list([]), settings: Value::map([]) },
                "r1",
                &tx,
            )
            .await
            .unwrap_err();
        assert!(error.contains("no credential"), "{error}");
    }

    #[test]
    fn a_tool_schema_is_translated_once_for_every_provider_that_wants_it() {
        let tools = Value::list([Value::map([
            ("name", Value::str("read_file")),
            ("description", Value::str("read it")),
            ("input_schema", Value::str(r#"{"type":"object","required":["path"]}"#)),
        ])]);
        let translated = tools_json(&tools).expect("tools");
        assert_eq!(translated[0]["function"]["name"], "read_file");
        // A schema arrives as a string and reaches a provider as an object.
        assert_eq!(translated[0]["function"]["parameters"]["required"][0], "path");
        assert!(tools_json(&Value::list([])).is_none());
    }

    #[test]
    fn a_call_without_a_name_is_dropped_because_no_tool_could_run_it() {
        let mut builder = CallBuilder::default();
        builder.args(0, r#"{"path":"x"}"#);
        assert_eq!(builder.finish().as_list().map(<[Value]>::len), Some(0));
    }

    #[test]
    fn a_mistyped_credential_is_noticed_by_its_shape() {
        assert!(credential_looks_wrong("/home/me/token.txt"));
        assert!(credential_looks_wrong("short"));
        assert!(!credential_looks_wrong("sk-ant-0123456789abcdef"));
    }

    /// A user message with the bytes of an attachment already in it, as the kernel leaves it.
    fn with_attachment(text: &str, media: &str, data: &str) -> Value {
        Value::map([
            ("role", Value::str("user")),
            ("text", Value::str(text)),
            (
                "attachments",
                Value::list([Value::map([
                    ("hash", Value::str("a".repeat(64))),
                    ("media", Value::str(media)),
                    ("len", Value::Int(4)),
                    ("data", Value::str(data)),
                ])]),
            ),
        ])
    }

    #[test]
    fn a_message_reaches_openai_where_openai_looks_for_it() {
        // The session's own shape has `text`; OpenAI reads `content`. Sending the first is
        // a request that answers as if the message were empty, which is the kind of bug a
        // test with a template provider would never see.
        let messages = Value::list([Value::map([("role", Value::str("user")), ("text", Value::str("hi"))])]);
        let shaped = openai_messages(&messages);
        assert_eq!(shaped[0]["role"], "user");
        assert_eq!(shaped[0]["content"], "hi");
        assert!(shaped[0].get("text").is_none(), "the session's own key leaked through");
    }

    #[test]
    fn an_attachment_becomes_an_image_a_provider_can_read() {
        let messages = Value::list([with_attachment("look", "image/png", "iVBORw0KGgo=")]);

        let openai = openai_messages(&messages);
        assert_eq!(openai[0]["content"][0]["type"], "text");
        assert_eq!(openai[0]["content"][0]["text"], "look");
        assert_eq!(openai[0]["content"][1]["type"], "image_url");
        assert_eq!(
            openai[0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,iVBORw0KGgo="
        );

        let anthropic = anthropic_messages(&messages);
        assert_eq!(anthropic[0]["content"][1]["type"], "image");
        assert_eq!(anthropic[0]["content"][1]["source"]["type"], "base64");
        assert_eq!(anthropic[0]["content"][1]["source"]["media_type"], "image/png");
        assert_eq!(anthropic[0]["content"][1]["source"]["data"], "iVBORw0KGgo=");
    }

    #[test]
    fn a_message_with_no_text_but_an_image_is_still_a_message() {
        // Text is optional when there are bytes: a person who pastes a screenshot and says
        // nothing has still said something.
        let message = Value::map([
            ("role", Value::str("user")),
            ("text", Value::str("")),
            (
                "attachments",
                Value::list([Value::map([
                    ("media", Value::str("image/png")),
                    ("data", Value::str("AAAA")),
                ])]),
            ),
        ]);
        let openai = openai_messages(&Value::list([message.clone()]));
        assert_eq!(openai[0]["content"][0]["type"], "image_url");
        assert_eq!(openai[0]["content"].as_array().map(Vec::len), Some(1), "an empty text part was sent");
        let anthropic = anthropic_messages(&Value::list([message]));
        assert_eq!(anthropic[0]["content"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn an_attachment_whose_bytes_are_gone_is_said_rather_than_dropped() {
        // Silence would be a lie: the model would answer as though nothing had been
        // attached. Both providers are told, in a text part, that something is missing.
        let message = Value::map([
            ("role", Value::str("user")),
            ("text", Value::str("see this")),
            (
                "attachments",
                Value::list([Value::map([
                    ("hash", Value::str("b".repeat(64))),
                    ("missing", Value::Bool(true)),
                ])]),
            ),
        ]);
        let openai = openai_messages(&Value::list([message.clone()]));
        assert!(openai[0]["content"][1]["text"].as_str().unwrap().contains("unavailable"));
        let anthropic = anthropic_messages(&Value::list([message]));
        assert!(anthropic[0]["content"][1]["text"].as_str().unwrap().contains("unavailable"));
    }

    #[test]
    fn tool_calls_and_results_go_where_each_provider_reads_them() {
        let messages = Value::list([
            Value::map([
                ("role", Value::str("assistant")),
                ("text", Value::str("")),
                (
                    "calls",
                    Value::list([Value::map([
                        ("id", Value::str("c1")),
                        ("name", Value::str("echo")),
                        ("args", Value::map([("text", Value::str("hi"))])),
                        ("result", Value::str("hi")),
                    ])]),
                ),
            ]),
            Value::map([
                ("role", Value::str("tool")),
                ("call", Value::str("c1")),
                ("text", Value::str("hi")),
            ]),
        ]);

        let openai = openai_messages(&messages);
        // An assistant message that is only a call has `content: null`, not an empty string.
        assert!(openai[0]["content"].is_null());
        assert_eq!(openai[0]["tool_calls"][0]["id"], "c1");
        assert_eq!(openai[0]["tool_calls"][0]["type"], "function");
        assert_eq!(openai[0]["tool_calls"][0]["function"]["name"], "echo");
        // Arguments are a JSON *string* on this wire, not an object.
        assert_eq!(openai[0]["tool_calls"][0]["function"]["arguments"], r#"{"text":"hi"}"#);
        assert_eq!(openai[1]["role"], "tool");
        assert_eq!(openai[1]["tool_call_id"], "c1");
        assert_eq!(openai[1]["content"], "hi");

        let anthropic = anthropic_messages(&messages);
        assert_eq!(anthropic[0]["content"][0]["type"], "tool_use");
        assert_eq!(anthropic[0]["content"][0]["input"]["text"], "hi");
        // A tool's answer is a `tool_result` block inside a user message.
        assert_eq!(anthropic[1]["role"], "user");
        assert_eq!(anthropic[1]["content"][0]["type"], "tool_result");
        assert_eq!(anthropic[1]["content"][0]["tool_use_id"], "c1");
    }

    #[test]
    fn a_system_prompt_is_a_message_to_one_provider_and_a_field_to_the_other() {
        let messages = Value::list([
            Value::map([("role", Value::str("system")), ("text", Value::str("be brief"))]),
            Value::map([("role", Value::str("user")), ("text", Value::str("hi"))]),
        ]);
        assert_eq!(openai_messages(&messages)[0]["role"], "system");
        assert_eq!(openai_messages(&messages)[0]["content"], "be brief");
        assert_eq!(system_prompt(&messages).as_deref(), Some("be brief"));
        // And Anthropic's list has no system message in it at all.
        assert_eq!(anthropic_messages(&messages).as_array().map(Vec::len), Some(1));
        assert_eq!(system_prompt(&Value::list([])), None);
    }

    #[test]
    fn a_provider_is_told_about_bytes_the_kernel_resolved() {
        // The kernel's own resolution: a hash becomes base64, and a hash with no bytes
        // becomes a message saying so. `provider` and `kernel` are tested apart because
        // they are apart: this is the seam between them.
        let blobs = Arc::new(crate::Blobs::in_memory());
        let stored = blobs.put(&[0x89, b'P', b'N', b'G', 1, 2], None).expect("a blob");
        let kernel = crate::Daemon::new(ScriptedProvider::always("hi")).with_blobs(blobs);
        let messages = Value::list([Value::map([
            ("role", Value::str("user")),
            ("text", Value::str("look")),
            (
                "attachments",
                Value::list([
                    Value::map([("hash", Value::str(&stored.hash))]),
                    Value::map([("hash", Value::str("0".repeat(64)))]),
                ]),
            ),
        ])]);

        let resolved = kernel.with_attachments(messages);
        let attachments = resolved
            .index(0)
            .and_then(|message| message.get("attachments"))
            .and_then(Value::as_list)
            .expect("attachments");
        assert_eq!(attachments[0].get("media").and_then(Value::as_str), Some("image/png"));
        assert_eq!(attachments[0].get("data").and_then(Value::as_str), Some("iVBORwEC"));
        assert!(attachments[0].get("missing").is_none(), "a stored blob was marked missing");
        assert_eq!(attachments[1].get("missing").and_then(Value::as_bool), Some(true));
        assert!(attachments[1].get("data").is_none());
    }
    #[tokio::test]
    async fn a_responses_stream_names_its_items_and_is_not_chat_completions() {
        // The whole point of a second OpenAI-shaped adapter: `input` items rather
        // than `messages`, an event per item, and a completed call that replaces the
        // fragments streamed before it.
        let server = SseServer::start(
            vec![
                event(r#"{"type":"response.output_text.delta","delta":"Hel"}"#),
                event(r#"{"type":"response.output_text.delta","delta":"lo"}"#),
                event(r#"{"type":"response.reasoning_summary_text.delta","delta":"thinking"}"#),
                event(r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"fc_1","name":"read_file"}}"#),
                event(r#"{"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"path\":"}"#),
                event(r#"{"type":"response.function_call_arguments.delta","output_index":1,"delta":"\"a.rs\"}"}"#),
                event(r#"{"type":"response.output_item.done","output_index":1,"item":{"type":"function_call","call_id":"fc_1","name":"read_file","arguments":"{\"path\":\"a.rs\"}"}}"#),
                event(r#"{"type":"response.completed","response":{"usage":{"input_tokens":9,"output_tokens":4},"output":[{"type":"function_call","call_id":"fc_1","name":"read_file","arguments":"{\"path\":\"a.rs\"}"}]}}"#),
            ],
            "200 OK",
        )
        .await;
        let provider = OpenAiResponses::new("openai-codex", format!("http://{}", server.address), http());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let answer = provider
            .stream(
                &ProviderRequest {
                    model: "gpt".into(),
                    messages: Value::list([
                        Value::map([("role", Value::str("system")), ("text", Value::str("be brief"))]),
                        Value::map([("role", Value::str("user")), ("text", Value::str("hi"))]),
                    ]),
                    tools: Value::list([]),
                    settings: Value::map([("max_tokens", Value::Int(512))]),
                },
                "r1",
                &tx,
            )
            .await
            .expect("a stream");
        assert_eq!(answer.text, "Hello");
        assert_eq!(answer.thinking, "thinking");
        assert_eq!(answer.input_tokens, 9);
        assert_eq!(answer.output_tokens, 4);
        let calls = answer.tool_calls.as_list().expect("calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].get("id").and_then(Value::as_str), Some("fc_1"));
        assert_eq!(calls[0].get("name").and_then(Value::as_str), Some("read_file"));
        assert_eq!(
            calls[0].get("args").and_then(|args| args.get("path")).and_then(Value::as_str),
            Some("a.rs"),
            "a finished call's arguments replace the fragments: {calls:?}"
        );
        // Both kinds of stream event reach the client, and on their own channels, so
        // a frontend can draw thinking differently from text.
        let mut saw_delta = false;
        let mut saw_thinking = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                KernelEvent::ProviderDelta { .. } => saw_delta = true,
                KernelEvent::ProviderThinking { .. } => saw_thinking = true,
                _ => {}
            }
        }
        assert!(saw_delta, "a text delta was not reported");
        assert!(saw_thinking, "reasoning was not reported as thinking");

        let requests = server.requests().await;
        let body = &requests[0];
        // The api's own names: `/responses`, `instructions` for the system prompt,
        // `input` for the messages, and `max_output_tokens` for the budget.
        assert!(requests[0].contains("POST /responses "), "{body}");
        assert!(body.contains(r#""instructions":"be brief""#), "{body}");
        assert!(body.contains(r#""input":"#), "{body}");
        assert!(!body.contains(r#""messages""#), "{body}");
        assert!(body.contains(r#""max_output_tokens":512"#), "{body}");
        assert!(!body.contains(r#""max_tokens""#), "{body}");
        assert!(body.contains(r#""text":"hi","type":"input_text""#), "{body}");
        // The one credential slot, sent as a bearer token like the chat api.
        assert!(body.contains(r#""store":false"#), "{body}");
    }

    #[test]
    fn a_finished_responses_call_is_usable_and_an_unfinished_one_is_not() {
        // A call with no name is one no tool can run and no result can answer.
        let finished = PartialCall { id: "fc_1".into(), name: "echo".into(), args: "{}".into() };
        assert!(finished.usable());
        assert_eq!(finished.to_value().get("name").and_then(Value::as_str), Some("echo"));
        assert!(!PartialCall::default().usable());
    }
}
