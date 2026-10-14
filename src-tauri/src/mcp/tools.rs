// MCP tool implementations.
//
// Every tool call flows through `call_tool`, which resolves the space's MCP
// access level, gates reads (`policy.rs`), executes through the same
// `QueryEngine` the UI uses, and writes an audit row — including denials.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::driver::DatabaseDriver;
use crate::db::{DriverKind, QueryResult};
use crate::mcp::policy::Access;
use crate::mcp::McpState;
use crate::storage::mcp_log::{make_entry, STATUS_DENIED, STATUS_ERROR, STATUS_OK};
use crate::storage::Space;

/// Rows returned per result set when the agent does not ask for a limit.
pub const DEFAULT_ROW_LIMIT: usize = 200;
/// Hard ceiling on rows per result set for MCP callers.
pub const MAX_ROW_LIMIT: usize = 1000;
/// Hard cap per statement. On expiry the future is dropped, which triggers the
/// drivers' cancel-on-drop behavior server-side.
const STATEMENT_TIMEOUT: Duration = Duration::from_secs(120);
/// Longest statement snippet included in a denial message.
const SNIPPET_MAX_CHARS: usize = 300;

/// Outcome of resolving a tool call's connection: the space, its access level,
/// and the live driver.
struct Resolved {
    space: Space,
    access: Access,
    driver: Arc<dyn DatabaseDriver>,
}

/// Tool execution failure. `denied` marks policy refusals (audit status
/// `denied`); everything else audits as `error`.
pub(crate) struct ToolFail {
    pub(crate) message: String,
    denied: bool,
}

impl ToolFail {
    fn deny(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            denied: true,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            denied: false,
        }
    }
}

/// Dispatch a tools/call. `Ok(result)` is a complete MCP CallToolResult;
/// `Err(message)` means the request itself was malformed (unknown tool).
pub(crate) async fn call_tool(
    state: &McpState,
    name: &str,
    args: Option<&Value>,
) -> Result<Value, ToolFail> {
    let started = Instant::now();
    let args: Value = args.cloned().unwrap_or(Value::Null);

    let outcome = match name {
        "list_connections" => list_connections(state).await,
        "list_databases" => match resolve_args(state, &args).await {
            Ok((resolved, rest)) => list_databases(state, resolved, rest).await,
            Err(fail) => Err(fail),
        },
        "get_schema" => match resolve_args(state, &args).await {
            Ok((resolved, rest)) => get_schema(state, resolved, rest).await,
            Err(fail) => Err(fail),
        },
        "run_query" => match resolve_args(state, &args).await {
            Ok((resolved, rest)) => run_query(state, resolved, rest).await,
            Err(fail) => Err(fail),
        },
        other => return Err(ToolFail::error(format!("unknown tool: {other}"))),
    };

    let (payload, status) = match outcome {
        Ok(v) => (v, STATUS_OK),
        Err(fail) => (
            json!({ "error": fail.message }),
            if fail.denied {
                STATUS_DENIED
            } else {
                STATUS_ERROR
            },
        ),
    };
    let is_error = status != STATUS_OK;

    // Audit every call, including failures. Logging must not break the tool.
    log_call(state, name, &args, status, started, &payload).await;

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&payload).unwrap_or_default()
            }
        ],
        "isError": is_error,
    }))
}

/// Extract the `connection` argument shared by every tool except
/// `list_connections`, resolve it, and return with the remaining arguments.
async fn resolve_args(state: &McpState, args: &Value) -> Result<(Resolved, Value), ToolFail> {
    let obj = args
        .as_object()
        .ok_or_else(|| ToolFail::error("arguments must be a JSON object"))?;
    let connection = obj
        .get("connection")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFail::error("missing required string argument: connection"))?
        .to_string();

    let resolved = resolve_connection(state, &connection).await?;
    Ok((resolved, args.clone()))
}

/// Look up the space (connection id == space id), enforce its access level,
/// and make sure the connection is registered and live.
async fn resolve_connection(state: &McpState, connection_id: &str) -> Result<Resolved, ToolFail> {
    let space = {
        let db = state.db.lock().await;
        db.get_space(connection_id)
            .map_err(|e| ToolFail::error(format!("storage error: {e}")))?
    };
    let Some(space) = space else {
        return Err(ToolFail::deny(format!(
            "unknown connection '{connection_id}'"
        )));
    };

    let access = Access::parse(&space.mcp_access);
    if access.is_off() {
        return Err(ToolFail::deny(format!(
            "space '{}' is not exposed to MCP (access level 'off')",
            space.name
        )));
    }
    if !space.has_connection() {
        return Err(ToolFail::error(format!(
            "space '{}' has no database connection",
            space.name
        )));
    }

    match state.connections.resolve(connection_id).await {
        Err(_) => {
            // Never registered this session (space created before the MCP
            // request): build the config from the saved space and connect.
            let password = {
                let db = state.db.lock().await;
                db.get_space_password(connection_id)
                    .map_err(|e| ToolFail::error(format!("storage error: {e}")))?
                    .unwrap_or_default()
            };
            let config = crate::commands::connection_config_for_space(&space, password)
                .map_err(|e| ToolFail::error(e.to_string()))?;
            let _ = state.connections.add_connection(config).await;
            state
                .connections
                .connect(connection_id)
                .await
                .map_err(|e| {
                    ToolFail::error(format!("failed to connect to '{}': {e}", space.name))
                })?;
        }
        Ok((driver, _)) => {
            if !driver.is_connected(connection_id).await {
                state
                    .connections
                    .connect(connection_id)
                    .await
                    .map_err(|e| {
                        ToolFail::error(format!("failed to connect to '{}': {e}", space.name))
                    })?;
            }
        }
    }

    let (driver, _config) = state
        .connections
        .resolve(connection_id)
        .await
        .map_err(|e| ToolFail::error(e.to_string()))?;

    Ok(Resolved {
        space,
        access,
        driver,
    })
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

/// Strip credentials from a URL-shaped host. MongoDB spaces store their whole
/// connection string (including `user:password@`) in the host field — the MCP
/// surface must never echo that back to the agent. Plain `host:port` and
/// scheme-less values pass through untouched.
fn redact_host(host: &str) -> String {
    let Some(scheme_end) = host.find("://") else {
        return host.to_string();
    };
    let authority_start = scheme_end + 3;
    let authority_end = host[authority_start..]
        .find('/')
        .map(|i| authority_start + i)
        .unwrap_or(host.len());
    let authority = &host[authority_start..authority_end];
    // Last '@' in the authority: passwords may themselves contain '@'.
    match authority.rfind('@') {
        Some(at) => {
            let mut out = String::with_capacity(host.len());
            out.push_str(&host[..authority_start]);
            out.push_str("***");
            out.push_str(&authority[at..]);
            out.push_str(&host[authority_end..]);
            out
        }
        None => host.to_string(),
    }
}

/// Spaces with a connection that are exposed to MCP. Never includes credentials.
async fn list_connections(state: &McpState) -> Result<Value, ToolFail> {
    let spaces = {
        let db = state.db.lock().await;
        db.get_all_spaces()
            .map_err(|e| ToolFail::error(format!("storage error: {e}")))?
    };

    let mut out = Vec::new();
    for space in spaces {
        let access = Access::parse(&space.mcp_access);
        if access.is_off() || !space.has_connection() {
            continue;
        }
        out.push(json!({
            "id": space.id,
            "name": space.name,
            "driver": space.connection_driver,
            "host": space.connection_host.as_deref().map(redact_host),
            "port": space.connection_port,
            "database": space.connection_database,
            "mcp_access": access.as_str(),
            "connected": match state.connections.resolve(&space.id).await {
                Ok((driver, _)) => driver.is_connected(&space.id).await,
                Err(_) => false,
            },
        }));
    }
    Ok(json!({ "connections": out }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::redact_host;

    #[test]
    fn redacts_userinfo_in_mongo_connection_strings() {
        assert_eq!(
            redact_host(
                "mongodb+srv://dev_legacy:hunter2@nonprod-pl-0.zwkkc.mongodb.net/dev-au_qbe"
            ),
            "mongodb+srv://***@nonprod-pl-0.zwkkc.mongodb.net/dev-au_qbe"
        );
        assert_eq!(
            redact_host("mongodb://u:p@h1:27017/db?replicaSet=rs0"),
            "mongodb://***@h1:27017/db?replicaSet=rs0"
        );
    }

    #[test]
    fn redacts_passwords_containing_at_signs() {
        assert_eq!(redact_host("mongodb://u:p@ss@h/db"), "mongodb://***@h/db");
    }

    #[test]
    fn leaves_plain_hosts_untouched() {
        assert_eq!(redact_host("localhost:1433"), "localhost:1433");
        assert_eq!(
            redact_host("https://example.com/path"),
            "https://example.com/path"
        );
        assert_eq!(redact_host(""), "");
    }
}

async fn list_databases(
    state: &McpState,
    resolved: Resolved,
    _args: Value,
) -> Result<Value, ToolFail> {
    let entries = state
        .connections
        .get_databases_with_access(&resolved.space.id)
        .await
        .map_err(|e| ToolFail::error(e.to_string()))?;
    let databases: Vec<Value> = entries
        .iter()
        .map(|e| json!({ "name": e.name, "has_access": e.has_access }))
        .collect();
    Ok(json!({ "databases": databases }))
}

#[derive(Deserialize)]
struct SchemaArgs {
    database: String,
    #[serde(default)]
    table: Option<String>,
}

/// Cached schema (fetched on miss), optionally narrowed to one table.
async fn get_schema(state: &McpState, resolved: Resolved, args: Value) -> Result<Value, ToolFail> {
    let args: SchemaArgs = serde_json::from_value(args)
        .map_err(|e| ToolFail::error(format!("invalid arguments: {e}")))?;

    let schema = match state
        .schema_manager
        .get_cached_schema(&resolved.space.id, &args.database)
        .await
    {
        Some(cached) => cached,
        None => state
            .schema_manager
            .fetch_schema(&resolved.space.id, &args.database, None)
            .await
            .map_err(|e| ToolFail::error(e.to_string()))?,
    };

    match args.table {
        None => serde_json::to_value(&schema).map_err(|e| ToolFail::error(e.to_string())),
        Some(table) => {
            let mut filtered = schema;
            filtered
                .tables
                .retain(|t| t.table_name.eq_ignore_ascii_case(&table));
            if filtered.tables.is_empty() {
                return Err(ToolFail::error(format!(
                    "table '{table}' not found in database '{}'",
                    args.database
                )));
            }
            filtered.relationships.retain(|r| {
                r.source_table_name.eq_ignore_ascii_case(&table)
                    || r.target_table_name.eq_ignore_ascii_case(&table)
            });
            serde_json::to_value(&filtered).map_err(|e| ToolFail::error(e.to_string()))
        }
    }
}

#[derive(Deserialize)]
struct RunQueryArgs {
    query: String,
    #[serde(default)]
    database: Option<String>,
    #[serde(default)]
    row_limit: Option<usize>,
}

async fn run_query(state: &McpState, resolved: Resolved, args: Value) -> Result<Value, ToolFail> {
    let args: RunQueryArgs = serde_json::from_value(args)
        .map_err(|e| ToolFail::error(format!("invalid arguments: {e}")))?;
    if args.query.trim().is_empty() {
        return Err(ToolFail::error("query must not be empty"));
    }
    let row_limit = args
        .row_limit
        .unwrap_or(DEFAULT_ROW_LIMIT)
        .clamp(1, MAX_ROW_LIMIT);

    let read_only = resolved.access.is_read();

    // Read-level gate. Postgres relies on engine-level enforcement instead;
    // Mongo's undecidable statements (SQL subset / unparseable) go to the driver.
    if read_only {
        let kind: DriverKind = resolved
            .space
            .connection_driver
            .parse()
            .map_err(|e| ToolFail::error(format!("unknown driver: {e}")))?;
        for unit in resolved.driver.split_statements(&args.query) {
            if unit.trim().is_empty() {
                continue;
            }
            let allowed = match kind {
                DriverKind::Mssql => crate::mcp::policy::mssql_read_allowed(&unit),
                DriverKind::Mongodb => {
                    crate::mcp::policy::mongo_read_allowed(&unit).unwrap_or(true)
                }
                DriverKind::Postgres => true,
            };
            if !allowed {
                return Err(ToolFail::deny(format!(
                    "statement refused: access level is 'read' and this statement is not \
                     provably read-only: \"{}\". Use a connection with 'write' access, or a \
                     read-only login for hard guarantees.",
                    snippet(&unit, SNIPPET_MAX_CHARS)
                )));
            }
        }
    }

    // Human gate: every execution on a write-access space needs a click in the
    // running Larik window. Read-only spaces skip this — they are enforced
    // read-only server-side, and the user opted them in deliberately.
    if !read_only {
        let request = crate::mcp::approvals::ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            connection_id: resolved.space.id.clone(),
            space_name: resolved.space.name.clone(),
            driver: resolved.space.connection_driver.clone(),
            statement: snippet(&args.query, 4000),
            access_level: "write".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        match crate::mcp::approvals::request_approval(
            state,
            request,
            crate::mcp::approvals::APPROVAL_TIMEOUT,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => {
                return Err(ToolFail::deny(
                    "statement denied by the user in Larik's approval prompt",
                ));
            }
            Err(msg) => return Err(ToolFail::deny(msg)),
        }
    }

    let fut = state.query_engine.execute_query(
        &resolved.space.id,
        &args.query,
        args.database.as_deref(),
        false,
        Some(row_limit),
        read_only,
    );
    let results = match tokio::time::timeout(STATEMENT_TIMEOUT, fut).await {
        Err(_) => {
            return Err(ToolFail::error(format!(
                "statement timed out after {}s and was cancelled",
                STATEMENT_TIMEOUT.as_secs()
            )));
        }
        Ok(Err(e)) => return Err(ToolFail::error(format!("query failed: {e}"))),
        Ok(Ok(results)) => results,
    };

    let mut any_error = false;
    let result_sets: Vec<Value> = results
        .iter()
        .map(|r| {
            if r.error.is_some() {
                any_error = true;
            }
            result_to_json(r)
        })
        .collect();

    let mut payload = json!({ "results": result_sets });
    if let Some(g) = results.iter().find_map(|r| r.generated_query.clone()) {
        payload["generated_query"] = Value::String(g);
    }
    // Partial failures inside a batch (one statement of many errored) surface
    // as an error result carrying the per-statement details.
    if any_error {
        return Err(ToolFail::error(
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "query failed".to_string()),
        ));
    }
    Ok(payload)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn result_to_json(r: &QueryResult) -> Value {
    json!({
        "columns": r.columns,
        "rows": r.rows,
        "row_count": r.row_count,
        "truncated": r.truncated,
        "limit_applied": r.limit_applied,
        "execution_time_ms": r.execution_time_ms,
        "statement": r.statement_text,
        "error": r.error,
    })
}

fn snippet(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars).collect();
        format!("{cut}…")
    }
}

/// Write the audit row for one tool call. Best effort — never fails the call.
async fn log_call(
    state: &McpState,
    tool: &str,
    args: &Value,
    status: &str,
    started: Instant,
    payload: &Value,
) {
    let connection = args
        .get("connection")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let statement = match args.get("query").and_then(Value::as_str) {
        Some(query) => query.to_string(),
        None => args.to_string(),
    };

    let (space_name, access_level) = {
        let db = state.db.lock().await;
        match db.get_space(&connection) {
            Ok(Some(space)) => {
                let a = Access::parse(&space.mcp_access);
                (space.name, a.as_str().to_string())
            }
            _ => (String::new(), "off".to_string()),
        }
    };

    let rows_returned = payload
        .get("results")
        .and_then(Value::as_array)
        .map(|sets| {
            sets.iter()
                .filter_map(|s| s.get("row_count"))
                .filter_map(Value::as_i64)
                .sum()
        })
        .unwrap_or(0);

    let entry = make_entry(
        tool,
        &connection,
        &space_name,
        &statement,
        &access_level,
        status,
        started.elapsed().as_millis() as i64,
        rows_returned,
    );
    let db = state.db.lock().await;
    if let Err(e) = db.insert_mcp_log(&entry) {
        eprintln!("[MCP] failed to write audit log: {e}");
    }
}
