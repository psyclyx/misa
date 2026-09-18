//! The narrow MCP stdio boundary used by CLI-backed providers.
//!
//! MCP is transport here, not a second tool implementation. The daemon composes the tools,
//! this module exposes those same capabilities to a provider process, and the process never
//! gets access to the session or to credential state.

use std::collections::BTreeMap;
use std::sync::Arc;

use misa_value::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::Tool;

const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const MAX_LINE: usize = 1024 * 1024;

/// Serve one MCP JSON-RPC connection on stdin/stdout.
pub async fn serve_stdio(tools: Vec<Arc<dyn Tool>>, schemas: Value) -> Result<(), String> {
    let tools = tools
        .into_iter()
        .map(|tool| (tool.name().to_string(), tool))
        .collect::<BTreeMap<_, _>>();
    let schemas = schemas
        .as_list()
        .unwrap_or(&[])
        .iter()
        .filter_map(|item| {
            let name = item.get("name").and_then(Value::as_str)?.to_string();
            let description = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let input_schema = item
                .get("input_schema")
                .and_then(Value::as_str)
                .and_then(|schema| serde_json::from_str::<serde_json::Value>(schema).ok())
                .unwrap_or_else(|| serde_json::json!({"type": "object"}));
            Some((name, (description, input_schema)))
        })
        .collect::<BTreeMap<_, _>>();

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| format!("MCP input: {error}"))?
    {
        if line.len() > MAX_LINE {
            write_error(
                &mut stdout,
                serde_json::Value::Null,
                -32600,
                "request too large",
            )
            .await?;
            continue;
        }
        let request = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(request) => request,
            Err(_) => {
                write_error(&mut stdout, serde_json::Value::Null, -32700, "Parse error").await?;
                continue;
            }
        };
        let Some(object) = request.as_object() else {
            write_error(
                &mut stdout,
                serde_json::Value::Null,
                -32600,
                "Invalid Request",
            )
            .await?;
            continue;
        };
        let id = object.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = object
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if method.starts_with("notifications/") {
            continue;
        }
        match method {
            "initialize" => {
                let version = object
                    .get("params")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|params| params.get("protocolVersion"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(DEFAULT_PROTOCOL_VERSION);
                write_result(
                    &mut stdout,
                    id,
                    serde_json::json!({
                        "protocolVersion": version,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "misa", "version": env!("CARGO_PKG_VERSION")},
                    }),
                )
                .await?;
            }
            "ping" => write_result(&mut stdout, id, serde_json::json!({})).await?,
            "tools/list" => {
                let items = schemas
                    .iter()
                    .filter_map(|(name, (description, input_schema))| {
                        tools.contains_key(name).then(|| {
                            serde_json::json!({
                                "name": name,
                                "description": description,
                                "inputSchema": input_schema,
                            })
                        })
                    })
                    .collect::<Vec<_>>();
                write_result(&mut stdout, id, serde_json::json!({"tools": items})).await?;
            }
            "tools/call" => {
                let Some(params) = object.get("params").and_then(serde_json::Value::as_object)
                else {
                    write_error(&mut stdout, id, -32602, "Invalid params").await?;
                    continue;
                };
                let Some(name) = params.get("name").and_then(serde_json::Value::as_str) else {
                    write_error(&mut stdout, id, -32602, "Invalid params").await?;
                    continue;
                };
                let name = name.strip_prefix("mcp__misa__").unwrap_or(name);
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let Some(arguments) = json_value(&arguments) else {
                    write_tool_result(&mut stdout, id, "tool arguments are not an object", true)
                        .await?;
                    continue;
                };
                let Some(tool) = tools.get(name) else {
                    write_tool_result(&mut stdout, id, &format!("no tool named `{name}`"), true)
                        .await?;
                    continue;
                };
                match tool.run(&arguments).await {
                    Ok(text) => write_tool_result(&mut stdout, id, &text, false).await?,
                    Err(text) => write_tool_result(&mut stdout, id, &text, true).await?,
                }
            }
            _ => write_error(&mut stdout, id, -32601, "Method not found").await?,
        }
    }
    Ok(())
}

fn json_value(value: &serde_json::Value) -> Option<Value> {
    Some(match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(value) => Value::Bool(*value),
        serde_json::Value::Number(value) => value
            .as_i64()
            .map(Value::Int)
            .or_else(|| value.as_f64().map(Value::Float))?,
        serde_json::Value::String(value) => Value::str(value),
        serde_json::Value::Array(values) => {
            Value::list(values.iter().map(json_value).collect::<Option<Vec<_>>>()?)
        }
        serde_json::Value::Object(values) => Value::Map(Arc::new(
            values
                .iter()
                .map(|(key, value)| Some((key.clone(), json_value(value)?)))
                .collect::<Option<BTreeMap<_, _>>>()?,
        )),
    })
}

async fn write_result(
    stdout: &mut tokio::io::Stdout,
    id: serde_json::Value,
    result: serde_json::Value,
) -> Result<(), String> {
    write_json(
        stdout,
        serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}),
    )
    .await
}

async fn write_tool_result(
    stdout: &mut tokio::io::Stdout,
    id: serde_json::Value,
    text: &str,
    is_error: bool,
) -> Result<(), String> {
    write_result(
        stdout,
        id,
        serde_json::json!({
            "content": [{"type": "text", "text": text}],
            "isError": is_error,
        }),
    )
    .await
}

async fn write_error(
    stdout: &mut tokio::io::Stdout,
    id: serde_json::Value,
    code: i32,
    message: &str,
) -> Result<(), String> {
    write_json(stdout, serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})).await
}

async fn write_json(
    stdout: &mut tokio::io::Stdout,
    value: serde_json::Value,
) -> Result<(), String> {
    stdout
        .write_all(value.to_string().as_bytes())
        .await
        .map_err(|error| format!("MCP output: {error}"))?;
    stdout
        .write_all(b"\n")
        .await
        .map_err(|error| format!("MCP output: {error}"))?;
    stdout
        .flush()
        .await
        .map_err(|error| format!("MCP output: {error}"))
}
