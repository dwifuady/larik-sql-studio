//! Newline-delimited JSON-RPC 2.0 MCP stdio server loop.
//!
//! Only JSON-RPC messages are ever written to `stdout`; all diagnostics go to `stderr`.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::context::StudioContext;
use crate::tools;

const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

/// Runs the MCP JSON-RPC loop over arbitrary async `reader` and `writer` (stdio in production,
/// duplex streams in tests).
pub async fn run_transport<R, W>(
    ctx: Arc<StudioContext>,
    reader: R,
    writer: W,
) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let writer = Arc::new(Mutex::new(writer));
    let mut lines = reader.lines();
    let mut in_flight = tokio::task::JoinSet::new();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(err) => {
                write_json_line(
                    &writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": {
                            "code": -32700,
                            "message": format!("Parse error: {err}")
                        }
                    }),
                )
                .await;
                continue;
            }
        };

        let id = msg.get("id").cloned();
        let method = msg
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        // Notifications have no `id` and expect no reply.
        let Some(req_id) = id else {
            continue;
        };

        let ctx = ctx.clone();
        let writer = writer.clone();
        in_flight.spawn(async move {
            let reply = handle_request(&ctx, req_id, &method, &params).await;
            write_json_line(&writer, &reply).await;
        });
    }

    // Drain any in-flight tool calls cleanly before EOF exit.
    while in_flight.join_next().await.is_some() {}
    Ok(())
}

/// Runs the MCP server on process stdin/stdout.
pub async fn run_stdio(ctx: Arc<StudioContext>) -> anyhow::Result<()> {
    let reader = BufReader::new(tokio::io::stdin());
    let writer = tokio::io::stdout();
    run_transport(ctx, reader, writer).await
}

pub async fn handle_request(
    ctx: &StudioContext,
    id: Value,
    method: &str,
    params: &Value,
) -> Value {
    match method {
        "initialize" => {
            let requested_version = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(DEFAULT_PROTOCOL_VERSION);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": requested_version,
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    },
                    "serverInfo": {
                        "name": "larik-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": "Use list_connections first to find exposed database workspaces. Never invent table or column names; call list_databases and get_schema before writing SQL. Pass a one-sentence description on run_query (required on write workspaces)."
                }
            })
        }
        "ping" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {}
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": tools::tool_definitions()
            }
        }),
        "tools/call" => {
            let Some(tool_name) = params.get("name").and_then(|v| v.as_str()) else {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32602,
                        "message": "Missing tool 'name' in tools/call params."
                    }
                });
            };
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            match tools::call_tool(ctx, tool_name, &args).await {
                Ok(text) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [
                            { "type": "text", "text": text }
                        ],
                        "isError": false
                    }
                }),
                Err(err) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [
                            { "type": "text", "text": err.to_string() }
                        ],
                        "isError": true
                    }
                }),
            }
        }
        "resources/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "resources": [] }
        }),
        "prompts/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "prompts": [] }
        }),
        other => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {other}")
            }
        }),
    }
}

async fn write_json_line<W: AsyncWrite + Unpin>(writer: &Arc<Mutex<W>>, value: &Value) {
    let Ok(mut line) = serde_json::to_string(value) else {
        return;
    };
    line.push('\n');
    let mut w = writer.lock().await;
    let _ = w.write_all(line.as_bytes()).await;
    let _ = w.flush().await;
}
