// Streamable-HTTP MCP endpoint (JSON response mode).
//
// A single POST /mcp route speaks JSON-RPC 2.0: `initialize`, `ping`,
// `tools/list`, `tools/call`. Notifications (no `id`) get 202. This is the
// spec's JSON response mode — valid for tools-only servers, no SSE machinery.
// GET/DELETE return 405. No CORS headers are emitted, so browser JS cannot
// call this endpoint; loopback bind means no Windows firewall prompt.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::mcp::tools;
use crate::mcp::McpState;

/// MCP protocol versions this server can speak. A client's requested version
/// is echoed when supported; otherwise the newest one is offered.
const SUPPORTED_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
const DEFAULT_VERSION: &str = "2025-06-18";
/// Request body cap (1 MiB) — tool arguments are tiny; big bodies are hostile.
const MAX_BODY_BYTES: usize = 1_000_000;

/// Serve MCP on an already-bound listener until `shutdown` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: Arc<McpState>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) -> std::io::Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
        })
        .await
}

fn router(state: Arc<McpState>) -> Router {
    Router::new()
        .route(
            "/mcp",
            post(handle_post).get(not_allowed).delete(not_allowed),
        )
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

/// Constant-time bearer token check.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

async fn auth(State(state): State<Arc<McpState>>, req: Request, next: Next) -> Response {
    let expected = state.token.read().await;
    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    // Fail closed when no token is loaded: an empty expected token would
    // otherwise match an empty `Bearer ` header.
    let ok =
        !expected.is_empty() && provided.is_some_and(|p| ct_eq(p.as_bytes(), expected.as_bytes()));
    drop(expected);

    if ok {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

async fn not_allowed() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, "method not allowed").into_response()
}

#[derive(Deserialize)]
struct RpcRequest {
    #[serde(default = "default_jsonrpc")]
    #[allow(dead_code)] // validated implicitly: we always answer 2.0
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

fn default_jsonrpc() -> String {
    "2.0".to_string()
}

#[derive(Serialize)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

fn success(id: Value, result: Value) -> Response {
    Json(RpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    })
    .into_response()
}

fn failure(id: Value, code: i64, message: String) -> Response {
    Json(RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError { code, message }),
    })
    .into_response()
}

async fn handle_post(State(state): State<Arc<McpState>>, Json(rpc): Json<RpcRequest>) -> Response {
    // Notifications carry no id and expect no response body.
    let Some(id) = rpc.id else {
        return StatusCode::ACCEPTED.into_response();
    };

    match rpc.method.as_str() {
        "initialize" => success(id, initialize_result(&rpc.params)),
        "ping" => success(id, json!({})),
        "tools/list" => success(id, tools_list()),
        "tools/call" => {
            let name = rpc
                .params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let args = rpc.params.as_ref().and_then(|p| p.get("arguments"));
            match tools::call_tool(&state, &name, args).await {
                Ok(result) => success(id, result),
                Err(fail) => failure(id, -32602, fail.message),
            }
        }
        other => failure(id, -32601, format!("method not found: {other}")),
    }
}

fn initialize_result(params: &Option<Value>) -> Value {
    let requested = params
        .as_ref()
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = if SUPPORTED_VERSIONS.contains(&requested) {
        requested
    } else {
        DEFAULT_VERSION
    };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "Larik SQL Studio",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Larik SQL Studio exposes saved database connections. \
    Use list_connections first; only connections the user exposed to MCP are visible. \
    get_schema returns cached schema metadata; run_query executes through the same \
    engine as the UI and is read-only unless the connection has 'write' access."
    })
}

fn tool_descriptor(name: &str, description: &str, schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": schema })
}

fn tools_list() -> Value {
    json!({
        "tools": [
            tool_descriptor(
                "list_connections",
                "List database connections the user exposed to Larik's MCP server. \
    Returns id, name, driver, host, port, database and access level. Never credentials.",
                json!({ "type": "object", "properties": {} }),
            ),
            tool_descriptor(
                "list_databases",
                "List databases on a server with per-database access flags.",
                json!({
                    "type": "object",
                    "properties": { "connection": { "type": "string" } },
                    "required": ["connection"]
                }),
            ),
            tool_descriptor(
                "get_schema",
                "Schema metadata for a database: schemas, tables with columns, \
    relationships, routines. From the studio's schema cache. Pass `table` to narrow \
    to a single table (with its relationships).",
                json!({
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string" },
                        "database": { "type": "string" },
                        "table": { "type": "string" }
                    },
                    "required": ["connection", "database"]
                }),
            ),
            tool_descriptor(
                "run_query",
                "Run a SQL statement (or mongo-shell statement on MongoDB) against a \
    connection, same engine as the studio UI. Row limit defaults to 200, max 1000 per \
    result set; results are truncated with a flag. Read-only enforced server-side on \
    'read' connections (refused if not provably a read on MS-SQL/MongoDB; on PostgreSQL \
    session/transaction control such as SET or BEGIN is refused). On 'write' connections \
    every statement waits for the user's approval in the Larik window.",
                json!({
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string" },
                        "query": { "type": "string" },
                        "database": { "type": "string" },
                        "row_limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
                    },
                    "required": ["connection", "query"]
                }),
            ),
        ]
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Minimal HTTP/1.1 client for the service tests (no extra deps).
    async fn http_post(port: u16, path: &str, token: Option<&str>, body: &str) -> (u16, String) {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let auth = token
            .map(|t| format!("Authorization: Bearer {t}\r\n"))
            .unwrap_or_default();
        let req = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             {auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        let deadline = Duration::from_secs(5);
        let _ = tokio::time::timeout(deadline, stream.read_to_end(&mut buf)).await;
        let raw = String::from_utf8_lossy(&buf).to_string();
        let status: u16 = raw
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        (status, raw)
    }

    async fn spawn_server(state: Arc<McpState>) -> (u16, tokio::sync::oneshot::Sender<()>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = serve(listener, state, rx).await;
        });
        (port, tx)
    }

    fn test_state() -> Arc<McpState> {
        Arc::new(McpState::for_tests())
    }

    #[tokio::test]
    async fn rejects_missing_and_wrong_tokens() {
        let state = test_state();
        let (port, _tx) = spawn_server(state).await;
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

        let (status, _) = http_post(port, "/mcp", None, body).await;
        assert_eq!(status, 401);

        let (status, _) = http_post(port, "/mcp", Some("wrong"), body).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn empty_server_token_rejects_empty_bearer() {
        let state = test_state();
        *state.token.write().await = String::new();
        let (port, _tx) = spawn_server(state).await;
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

        let (status, _) = http_post(port, "/mcp", Some(""), body).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn initialize_and_tools_list() {
        let state = test_state();
        let (port, _tx) = spawn_server(state).await;
        let token = "tok";

        let (status, resp) = http_post(
            port,
            "/mcp",
            Some(token),
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        )
        .await;
        assert_eq!(status, 200);
        assert!(resp.contains("2025-03-26"), "echoes client version: {resp}");

        let (status, resp) = http_post(
            port,
            "/mcp",
            Some(token),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )
        .await;
        assert_eq!(status, 200);
        for tool in [
            "list_connections",
            "list_databases",
            "get_schema",
            "run_query",
        ] {
            assert!(resp.contains(tool), "missing tool {tool}");
        }
    }

    #[tokio::test]
    async fn notifications_get_accepted_and_unknown_methods_404_code() {
        let state = test_state();
        let (port, _tx) = spawn_server(state).await;

        let (status, resp) = http_post(
            port,
            "/mcp",
            Some("tok"),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .await;
        assert_eq!(status, 202);
        assert!(resp.trim().ends_with("0") || !resp.contains("result"));

        let (status, resp) = http_post(
            port,
            "/mcp",
            Some("tok"),
            r#"{"jsonrpc":"2.0","id":9,"method":"resources/list"}"#,
        )
        .await;
        assert_eq!(status, 200);
        assert!(resp.contains("-32601"), "{resp}");
    }

    #[tokio::test]
    async fn get_is_method_not_allowed() {
        let state = test_state();
        let (port, _tx) = spawn_server(state).await;
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(b"GET /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer tok\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf)).await;
        let raw = String::from_utf8_lossy(&buf).to_string();
        assert!(raw.starts_with("HTTP/1.1 405"), "{raw}");
    }
}
