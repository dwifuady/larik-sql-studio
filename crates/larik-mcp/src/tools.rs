//! The five Larik MCP tools mirroring WinUI `Larik.Mcp/StudioTools.cs`:
//! - `list_connections`
//! - `list_databases`
//! - `get_schema`
//! - `run_query`
//! - `open_in_studio`

use std::time::Instant;

use anyhow::{Result, anyhow};
use larik_core::mcp::{
    MCP_DEFAULT_ROW_LIMIT, MCP_MAX_ROW_LIMIT, MCP_MAX_STATEMENT_CHARS, MCP_MAX_STATEMENT_DURATION,
    MCP_RESPONSE_MAX_BYTES, McpAccess, McpApprovalRequest, McpColumnPayload, McpOpenRequest,
    McpQueryPayload, McpResultSetPayload, mcp_approval_status, mcp_log_status, mcp_pipe,
    trim_response_budget,
};
use larik_core::run_log::run_status;
use larik_data::{DatabaseInfo, QueryExecutionOptions, RoutineKind, TableKind};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::approval_channel;
use crate::context::StudioContext;

/// Returns the MCP `tools/list` definitions with exact names, JSON schemas, and annotations.
pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "list_connections",
            "description": "Lists the database connections the user has exposed to AI agents in Larik SQL Studio. Returns each connection's id, name, engine, endpoint, login and access level. Never returns passwords. The id is what every other tool takes as 'connection'.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        },
        {
            "name": "list_databases",
            "description": "Lists the databases on one connection, marking the ones the saved login cannot open. Pass 'database' to get_schema and run_query once you know which one matters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": {
                        "type": "string",
                        "description": "Connection id from list_connections."
                    }
                },
                "required": ["connection"],
                "additionalProperties": false
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        },
        {
            "name": "get_schema",
            "description": "Returns tables and views with column names and types, primary keys and foreign keys, so joins are written against real relationships instead of guessed ones. Pass 'table' to narrow to one table (name or schema.name), or 'include_routines' for stored procedures and functions with their parameters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": {
                        "type": "string",
                        "description": "Connection id from list_connections."
                    },
                    "database": {
                        "type": "string",
                        "description": "Database name from list_databases."
                    },
                    "table": {
                        "type": "string",
                        "description": "Optional table filter: 'orders' or 'dbo.orders'."
                    },
                    "include_routines": {
                        "type": "boolean",
                        "description": "Include stored procedures and functions. Default false."
                    }
                },
                "required": ["connection", "database"],
                "additionalProperties": false
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        },
        {
            "name": "run_query",
            "description": "Runs SQL on an exposed connection and returns result sets (columns, rows, a per-set 'truncated' flag) plus engine messages. Read-only workspaces are enforced by the server: writes and session control are refused there. Write workspaces require the user's approval in Larik. Always pass 'description' - one sentence on why this query is being run; a write workspace refuses a call without it. Pass 'reference' with the ticket or incident the work belongs to (for example 'PROJ-123') so the run is traceable in History.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": {
                        "type": "string",
                        "description": "Connection id from list_connections."
                    },
                    "query": {
                        "type": "string",
                        "description": "The SQL to run."
                    },
                    "database": {
                        "type": "string",
                        "description": "Database to run against; defaults to the connection's saved database."
                    },
                    "row_limit": {
                        "type": "integer",
                        "description": "Rows per result set: 200 by default, 1000 maximum.",
                        "minimum": 1,
                        "maximum": 1000
                    },
                    "description": {
                        "type": "string",
                        "description": "One sentence on why this query is needed. Required on a read-and-write workspace; shown in the user's approval prompt and kept in run history."
                    },
                    "reference": {
                        "type": "string",
                        "description": "Optional ticket, incident or branch this work belongs to, e.g. 'PROJ-123'. Free-form; kept in run history so the run can be found again."
                    }
                },
                "required": ["connection", "query"],
                "additionalProperties": false
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": false
            }
        },
        {
            "name": "open_in_studio",
            "description": "Opens a statement in Larik SQL Studio as a new query tab on that connection, so the user can review, adjust or run the agent's work themselves. Needs the studio to be running; fails with the reason when it is not. Nothing is executed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": {
                        "type": "string",
                        "description": "Connection id from list_connections."
                    },
                    "query": {
                        "type": "string",
                        "description": "The SQL to place in the tab."
                    },
                    "database": {
                        "type": "string",
                        "description": "Database for the tab; defaults to the connection's saved database."
                    }
                },
                "required": ["connection", "query"],
                "additionalProperties": false
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }
    ])
}

/// Dispatches a `tools/call` invocation and returns the JSON text payload (or an error message).
pub async fn call_tool(ctx: &StudioContext, name: &str, args: &Value) -> Result<String> {
    match name {
        "list_connections" => list_connections(ctx),
        "list_databases" => list_databases(ctx, args).await,
        "get_schema" => get_schema(ctx, args).await,
        "run_query" => run_query(ctx, args).await,
        "open_in_studio" => open_in_studio(ctx, args).await,
        other => Err(anyhow!("Unknown tool '{other}'.")),
    }
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing required string argument '{key}'."))
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn opt_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn opt_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(|v| v.as_i64())
        .map(|n| n.max(0) as usize)
}

fn list_connections(ctx: &StudioContext) -> Result<String> {
    let started = Instant::now();
    let spaces = match ctx.exposed_spaces() {
        Ok(s) => s,
        Err(err) => {
            ctx.audit_global(
                "list_connections",
                mcp_log_status::DENIED,
                Some(err.to_string()),
                started.elapsed(),
                0,
            );
            return Err(err);
        }
    };

    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::with_capacity(spaces.len());
    for space in spaces {
        if !seen.insert(space.id.clone()) {
            continue;
        }
        let provider = ctx.catalog.find(&space.provider_id);
        let provider_name = provider
            .as_ref()
            .map(|p| p.display_name().to_string())
            .unwrap_or_else(|| space.provider_id.clone());
        let endpoint = provider
            .as_ref()
            .and_then(|p| p.describe_endpoint(&space.visible_profile));
        let identity = provider
            .as_ref()
            .and_then(|p| p.describe_identity(&space.visible_profile));

        let mut obj = serde_json::Map::new();
        obj.insert("id".into(), Value::String(space.id));
        obj.insert("name".into(), Value::String(space.name));
        obj.insert("provider".into(), Value::String(provider_name));
        if let Some(ep) = endpoint {
            obj.insert("endpoint".into(), Value::String(ep));
        }
        if let Some(ident) = identity {
            obj.insert("identity".into(), Value::String(ident));
        }
        obj.insert(
            "access".into(),
            Value::String(space.access.as_storage().to_string()),
        );
        items.push(Value::Object(obj));
    }

    ctx.audit_global(
        "list_connections",
        mcp_log_status::OK,
        None,
        started.elapsed(),
        items.len(),
    );
    Ok(serde_json::to_string(&items)?)
}

async fn list_databases(ctx: &StudioContext, args: &Value) -> Result<String> {
    let started = Instant::now();
    let connection = req_str(args, "connection")?;
    let (space, provider) = ctx.require_space("list_databases", connection)?;

    let res = ctx
        .with_connection(&space, &provider, |mut conn| async move {
            let dbs = conn.list_databases().await;
            (conn, dbs)
        })
        .await;

    match res {
        Ok(mut dbs) => {
            DatabaseInfo::sort_display_order(&mut dbs);
            let payload: Vec<Value> = dbs
                .iter()
                .map(|db| {
                    json!({
                        "name": db.name,
                        "accessible": db.is_accessible
                    })
                })
                .collect();
            ctx.audit(
                "list_databases",
                &space,
                None,
                mcp_log_status::OK,
                None,
                started.elapsed(),
                payload.len(),
                None,
            );
            Ok(serde_json::to_string(&payload)?)
        }
        Err(err) => {
            ctx.audit(
                "list_databases",
                &space,
                None,
                mcp_log_status::ERROR,
                Some(err.to_string()),
                started.elapsed(),
                0,
                None,
            );
            Err(err)
        }
    }
}

async fn get_schema(ctx: &StudioContext, args: &Value) -> Result<String> {
    let started = Instant::now();
    let connection = req_str(args, "connection")?;
    let database = req_str(args, "database")?.trim().to_string();
    if database.is_empty() {
        return Err(anyhow!("'database' is required for get_schema."));
    }
    let table_filter = opt_str(args, "table").map(str::to_string);
    let include_routines = opt_bool(args, "include_routines");

    let (space, provider) = ctx.require_space("get_schema", connection)?;

    let db_clone = database.clone();
    let res = ctx
        .with_connection(&space, &provider, move |mut conn| async move {
            let outcome = async {
                let tables = conn.get_schema(Some(&db_clone)).await?;
                let routines = if include_routines {
                    Some(conn.get_routines(Some(&db_clone)).await?)
                } else {
                    None
                };
                Ok((tables, routines))
            }
            .await;
            (conn, outcome)
        })
        .await;

    match res {
        Ok((tables, routines)) => {
            let filtered: Vec<_> = tables
                .into_iter()
                .filter(|t| {
                    let Some(filter) = table_filter.as_deref() else {
                        return true;
                    };
                    if t.name.eq_ignore_ascii_case(filter) {
                        return true;
                    }
                    let qualified = format!("{}.{}", t.schema, t.name);
                    qualified.eq_ignore_ascii_case(filter)
                })
                .collect();

            let table_json: Vec<Value> = filtered
                .iter()
                .map(|t| {
                    let cols: Vec<Value> = t
                        .columns
                        .iter()
                        .map(|c| {
                            let mut obj = serde_json::Map::new();
                            obj.insert("name".into(), Value::String(c.name.clone()));
                            if let Some(dt) = &c.data_type {
                                obj.insert("type".into(), Value::String(dt.clone()));
                            }
                            obj.insert("nullable".into(), Value::Bool(c.nullable));
                            Value::Object(obj)
                        })
                        .collect();
                    let fks: Vec<Value> = t
                        .foreign_keys
                        .iter()
                        .map(|fk| {
                            let mut refs = serde_json::Map::new();
                            if let Some(rs) = &fk.referenced_schema {
                                refs.insert("schema".into(), Value::String(rs.clone()));
                            }
                            refs.insert("table".into(), Value::String(fk.referenced_table.clone()));
                            refs.insert(
                                "column".into(),
                                Value::String(fk.referenced_column.clone()),
                            );
                            json!({
                                "column": fk.column,
                                "constraint": fk.constraint,
                                "references": Value::Object(refs)
                            })
                        })
                        .collect();
                    json!({
                        "schema": t.schema,
                        "name": t.name,
                        "kind": match t.kind { TableKind::Table => "table", TableKind::View => "view" },
                        "columns": cols,
                        "primaryKey": t.primary_key,
                        "foreignKeys": fks
                    })
                })
                .collect();

            let mut root = serde_json::Map::new();
            root.insert("database".into(), Value::String(database));
            root.insert("tables".into(), Value::Array(table_json));

            if let Some(routine_list) = routines {
                let r_json: Vec<Value> = routine_list
                    .iter()
                    .map(|r| {
                        let params_json: Vec<Value> = r
                            .parameters
                            .iter()
                            .map(|p| {
                                let mut obj = serde_json::Map::new();
                                obj.insert("name".into(), Value::String(p.name.clone()));
                                if let Some(dt) = &p.data_type {
                                    obj.insert("type".into(), Value::String(dt.clone()));
                                }
                                Value::Object(obj)
                            })
                            .collect();
                        json!({
                            "schema": r.schema,
                            "name": r.name,
                            "kind": match r.kind {
                                RoutineKind::Procedure => "procedure",
                                RoutineKind::Function => "function",
                            },
                            "parameters": params_json
                        })
                    })
                    .collect();
                root.insert("routines".into(), Value::Array(r_json));
            }

            ctx.audit(
                "get_schema",
                &space,
                table_filter.as_deref(),
                mcp_log_status::OK,
                None,
                started.elapsed(),
                filtered.len(),
                None,
            );
            Ok(serde_json::to_string(&Value::Object(root))?)
        }
        Err(err) => {
            ctx.audit(
                "get_schema",
                &space,
                table_filter.as_deref(),
                mcp_log_status::ERROR,
                Some(err.to_string()),
                started.elapsed(),
                0,
                None,
            );
            Err(err)
        }
    }
}

async fn run_query(ctx: &StudioContext, args: &Value) -> Result<String> {
    let started = Instant::now();
    let connection = req_str(args, "connection")?;
    let query = req_str(args, "query")?;
    let description = opt_str(args, "description").map(str::to_string);
    let reference = opt_str(args, "reference").map(str::to_string);
    let row_limit = opt_usize(args, "row_limit")
        .unwrap_or(MCP_DEFAULT_ROW_LIMIT)
        .clamp(1, MCP_MAX_ROW_LIMIT);

    let (space, provider) = ctx.require_space("run_query", connection)?;

    let target_db = opt_str(args, "database")
        .map(str::to_string)
        .or_else(|| {
            space
                .visible_profile
                .get_non_empty("database")
                .map(str::to_string)
        });

    if query.trim().is_empty() {
        let msg = "Query cannot be empty.".to_string();
        ctx.audit(
            "run_query",
            &space,
            Some(query),
            mcp_log_status::DENIED,
            Some(msg.clone()),
            started.elapsed(),
            0,
            None,
        );
        return Err(anyhow!(msg));
    }

    if query.chars().count() > MCP_MAX_STATEMENT_CHARS {
        let msg = format!(
            "Statement exceeds the {MCP_MAX_STATEMENT_CHARS}-character limit for AI agent queries."
        );
        ctx.audit(
            "run_query",
            &space,
            Some(query),
            mcp_log_status::DENIED,
            Some(msg.clone()),
            started.elapsed(),
            0,
            None,
        );
        return Err(anyhow!(msg));
    }

    let mut decision_surface: Option<String> = None;

    match space.access {
        McpAccess::Off => {
            let msg = format!("Workspace '{}' is not exposed to AI agents.", space.name);
            ctx.audit(
                "run_query",
                &space,
                Some(query),
                mcp_log_status::DENIED,
                Some(msg.clone()),
                started.elapsed(),
                0,
                None,
            );
            return Err(anyhow!(msg));
        }
        McpAccess::Read => {
            // Layer 1: fail-closed static statement classifier.
            let allowed = provider
                .read_only_policy()
                .is_some_and(|policy| policy.is_proven_read(query));
            if !allowed {
                let msg = format!(
                    "Refused: '{}' is configured as Read only for AI agents, and this statement could modify data or session state. Switch the workspace to Read and write in Settings -> AI agents if you want Larik to ask for approval on writes.",
                    space.name
                );
                ctx.audit(
                    "run_query",
                    &space,
                    Some(query),
                    mcp_log_status::DENIED,
                    Some(msg.clone()),
                    started.elapsed(),
                    0,
                    None,
                );
                return Err(anyhow!(msg));
            }
        }
        McpAccess::Write => {
            // Require a human-readable description before prompting the user.
            if description.is_none() {
                let msg = format!(
                    "Refused: '{}' is a Read and write workspace, so 'description' is required (one sentence explaining why this query is being run).",
                    space.name
                );
                ctx.audit(
                    "run_query",
                    &space,
                    Some(query),
                    mcp_log_status::DENIED,
                    Some(msg.clone()),
                    started.elapsed(),
                    0,
                    None,
                );
                return Err(anyhow!(msg));
            }

            let approval_id = Uuid::new_v4().simple().to_string();
            let _ = ctx
                .store
                .create_mcp_approval(&approval_id, &space.id, Some(&space.name), query);

            let req = McpApprovalRequest::new(
                approval_id.clone(),
                space.id.clone(),
                space.name.clone(),
                space.provider_id.clone(),
                query.to_string(),
                description.clone(),
                reference.clone(),
            );

            match approval_channel::request_approval(&ctx.pipe_name, &req).await {
                Some(decision) if decision.approved => {
                    let _ = ctx.store.resolve_mcp_approval(
                        &approval_id,
                        &decision.status,
                        Some(&decision.surface),
                    );
                    decision_surface = Some(decision.surface);
                }
                Some(decision) => {
                    let _ = ctx.store.resolve_mcp_approval(
                        &approval_id,
                        &decision.status,
                        Some(&decision.surface),
                    );
                    let reason = decision.reason.unwrap_or_else(|| {
                        "The user denied the statement in Larik SQL Studio.".to_string()
                    });
                    ctx.audit(
                        "run_query",
                        &space,
                        Some(query),
                        mcp_log_status::DENIED,
                        Some(reason.clone()),
                        started.elapsed(),
                        0,
                        Some(&decision.surface),
                    );
                    ctx.record_run(
                        &space,
                        target_db.as_deref(),
                        query,
                        run_status::REFUSED,
                        Some(reason.clone()),
                        started.elapsed(),
                        0,
                        Some(row_limit),
                        false,
                        description,
                        reference,
                    );
                    return Err(anyhow!(reason));
                }
                None => {
                    let _ = ctx.store.resolve_mcp_approval(
                        &approval_id,
                        mcp_approval_status::ORPHANED,
                        None,
                    );
                    let reason = format!(
                        "Refused: '{}' allows writes only with your approval, and Larik SQL Studio is not running. Open Larik SQL Studio and retry.",
                        space.name
                    );
                    ctx.audit(
                        "run_query",
                        &space,
                        Some(query),
                        mcp_log_status::DENIED,
                        Some(reason.clone()),
                        started.elapsed(),
                        0,
                        None,
                    );
                    ctx.record_run(
                        &space,
                        target_db.as_deref(),
                        query,
                        run_status::REFUSED,
                        Some(reason.clone()),
                        started.elapsed(),
                        0,
                        Some(row_limit),
                        false,
                        description,
                        reference,
                    );
                    return Err(anyhow!(reason));
                }
            }
        }
    }

    let read_only = space.access == McpAccess::Read;
    let exec_opts = QueryExecutionOptions {
        max_rows: row_limit,
        read_only: read_only.then_some(true),
    };
    let query_owned = query.to_string();
    let db_for_exec = target_db.clone();

    let exec_started = Instant::now();
    let timed = tokio::time::timeout(
        MCP_MAX_STATEMENT_DURATION,
        ctx.with_connection(&space, &provider, move |mut conn| async move {
            let res = conn
                .execute_batch(&query_owned, db_for_exec.as_deref(), &exec_opts)
                .await;
            (conn, res)
        }),
    )
    .await;

    let results = match timed {
        Ok(Ok(results)) => results,
        Ok(Err(err)) => {
            let reason = err.to_string();
            ctx.audit(
                "run_query",
                &space,
                Some(query),
                mcp_log_status::ERROR,
                Some(reason.clone()),
                exec_started.elapsed(),
                0,
                decision_surface.as_deref(),
            );
            ctx.record_run(
                &space,
                target_db.as_deref(),
                query,
                run_status::FAILED,
                Some(reason.clone()),
                exec_started.elapsed(),
                0,
                Some(row_limit),
                false,
                description,
                reference,
            );
            return Err(anyhow!(reason));
        }
        Err(_) => {
            let reason = format!(
                "Statement timed out after {} seconds (hard limit for AI agent queries).",
                MCP_MAX_STATEMENT_DURATION.as_secs()
            );
            ctx.audit(
                "run_query",
                &space,
                Some(query),
                mcp_log_status::ERROR,
                Some(reason.clone()),
                exec_started.elapsed(),
                0,
                decision_surface.as_deref(),
            );
            ctx.record_run(
                &space,
                target_db.as_deref(),
                query,
                run_status::FAILED,
                Some(reason.clone()),
                exec_started.elapsed(),
                0,
                Some(row_limit),
                false,
                description,
                reference,
            );
            return Err(anyhow!(reason));
        }
    };

    if let Some(failed) = results.iter().find(|r| r.is_error) {
        let reason = failed.message.clone();
        ctx.audit(
            "run_query",
            &space,
            Some(query),
            mcp_log_status::ERROR,
            Some(reason.clone()),
            exec_started.elapsed(),
            0,
            decision_surface.as_deref(),
        );
        ctx.record_run(
            &space,
            target_db.as_deref(),
            query,
            run_status::FAILED,
            Some(reason.clone()),
            exec_started.elapsed(),
            0,
            Some(row_limit),
            false,
            description,
            reference,
        );
        return Err(anyhow!(reason));
    }

    let mut total_rows = 0usize;
    let mut any_truncated = false;
    let mut all_messages = Vec::new();
    let mut result_sets = Vec::with_capacity(results.len());

    for res in results {
        total_rows = total_rows.saturating_add(res.rows.len());
        if res.is_truncated {
            any_truncated = true;
        }
        all_messages.extend(res.messages);
        result_sets.push(McpResultSetPayload {
            columns: res
                .columns
                .into_iter()
                .map(|c| McpColumnPayload {
                    name: c.name,
                    r#type: c.data_type,
                })
                .collect(),
            rows: res.rows,
            records_affected: res.records_affected,
            truncated: res.is_truncated,
            message: res.message,
            elapsed_ms: res.elapsed.as_millis().min(u64::MAX as u128) as u64,
        });
    }

    let mut payload = McpQueryPayload {
        connection: space.id.clone(),
        database: target_db.clone(),
        row_limit,
        result_sets,
        messages: all_messages,
        generated_query: None,
        truncated_for_size: false,
    };

    let json_text = trim_response_budget(&mut payload, MCP_RESPONSE_MAX_BYTES);
    let was_truncated = any_truncated || payload.truncated_for_size;

    ctx.audit(
        "run_query",
        &space,
        Some(query),
        mcp_log_status::OK,
        None,
        exec_started.elapsed(),
        total_rows,
        decision_surface.as_deref(),
    );
    ctx.record_run(
        &space,
        target_db.as_deref(),
        query,
        run_status::OK,
        None,
        exec_started.elapsed(),
        total_rows,
        Some(row_limit),
        was_truncated,
        description,
        reference,
    );

    Ok(json_text)
}

async fn open_in_studio(ctx: &StudioContext, args: &Value) -> Result<String> {
    let started = Instant::now();
    let connection = req_str(args, "connection")?;
    let query = req_str(args, "query")?;
    let (space, _provider) = ctx.require_space("open_in_studio", connection)?;

    let target_db = opt_str(args, "database")
        .map(str::to_string)
        .or_else(|| {
            space
                .visible_profile
                .get_non_empty("database")
                .map(str::to_string)
        });

    let req = McpOpenRequest::new(
        Uuid::new_v4().simple().to_string(),
        space.id.clone(),
        Some(space.name.clone()),
        query.to_string(),
        target_db.clone(),
    );

    match approval_channel::request_open_in_studio(&ctx.pipe_name, &req).await {
        Some(resp) if resp.ok => {
            ctx.audit(
                "open_in_studio",
                &space,
                Some(query),
                mcp_log_status::OK,
                None,
                started.elapsed(),
                0,
                Some(mcp_pipe::SURFACE_APP),
            );
            let mut obj = serde_json::Map::new();
            obj.insert("opened".into(), Value::Bool(true));
            obj.insert("connection".into(), Value::String(space.id));
            if let Some(db) = target_db {
                obj.insert("database".into(), Value::String(db));
            }
            obj.insert(
                "message".into(),
                Value::String(format!(
                    "Opened in {} as a new query tab. Nothing was run.",
                    space.name
                )),
            );
            Ok(serde_json::to_string(&Value::Object(obj))?)
        }
        Some(resp) => {
            let reason = resp
                .reason
                .unwrap_or_else(|| "Larik SQL Studio could not open the query tab.".to_string());
            ctx.audit(
                "open_in_studio",
                &space,
                Some(query),
                mcp_log_status::ERROR,
                Some(reason.clone()),
                started.elapsed(),
                0,
                Some(mcp_pipe::SURFACE_APP),
            );
            Err(anyhow!(reason))
        }
        None => {
            let reason = "Larik SQL Studio is not running. Open Larik SQL Studio to use open_in_studio.".to_string();
            ctx.audit(
                "open_in_studio",
                &space,
                Some(query),
                mcp_log_status::ERROR,
                Some(reason.clone()),
                started.elapsed(),
                0,
                None,
            );
            Err(anyhow!(reason))
        }
    }
}
