// Audit log for the embedded MCP server.
//
// Every MCP tool call — including denied ones — is recorded here so the user
// can see exactly what an agent did with their connections.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::{DatabaseManager, StorageResult};

/// One MCP tool call, as recorded in the `mcp_log` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpLogEntry {
    pub id: String,
    pub ts: String,
    pub tool: String,
    pub connection_id: String,
    pub space_name: String,
    /// Statement/tool argument text, truncated to [`STATEMENT_MAX_CHARS`].
    pub statement: String,
    /// Access level in effect for the call (`read` or `write`).
    pub access_level: String,
    /// `ok`, `error`, or `denied`.
    pub status: String,
    pub duration_ms: i64,
    pub rows_returned: i64,
}

/// Cap on how much statement text is stored per log row.
const STATEMENT_MAX_CHARS: usize = 10_000;

/// Log entry for a denied call where the tool name is known but no result exists.
pub const STATUS_OK: &str = "ok";
pub const STATUS_ERROR: &str = "error";
pub const STATUS_DENIED: &str = "denied";

/// Build a log entry, truncating oversized statement text.
#[allow(clippy::too_many_arguments)]
pub fn make_entry(
    tool: &str,
    connection_id: &str,
    space_name: &str,
    statement: &str,
    access_level: &str,
    status: &str,
    duration_ms: i64,
    rows_returned: i64,
) -> McpLogEntry {
    let mut statement = statement.to_string();
    if statement.chars().count() > STATEMENT_MAX_CHARS {
        statement = statement.chars().take(STATEMENT_MAX_CHARS).collect();
    }
    McpLogEntry {
        id: Uuid::new_v4().to_string(),
        ts: String::new(), // filled by the DB default
        tool: tool.to_string(),
        connection_id: connection_id.to_string(),
        space_name: space_name.to_string(),
        statement,
        access_level: access_level.to_string(),
        status: status.to_string(),
        duration_ms,
        rows_returned,
    }
}

impl DatabaseManager {
    /// Record one MCP tool call
    pub fn insert_mcp_log(&self, entry: &McpLogEntry) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO mcp_log (
                    id, tool, connection_id, space_name, statement,
                    access_level, status, duration_ms, rows_returned
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                params![
                    entry.id,
                    entry.tool,
                    entry.connection_id,
                    entry.space_name,
                    entry.statement,
                    entry.access_level,
                    entry.status,
                    entry.duration_ms,
                    entry.rows_returned,
                ],
            )?;
            Ok(())
        })
    }

    /// List recent MCP tool calls, newest first
    pub fn list_mcp_log(&self, limit: usize) -> StorageResult<Vec<McpLogEntry>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, ts, tool, connection_id, space_name, statement,
                       access_level, status, duration_ms, rows_returned
                FROM mcp_log
                ORDER BY ts DESC, id DESC
                LIMIT ?1
                "#,
            )?;

            let entries = stmt
                .query_map(params![limit], |row| {
                    Ok(McpLogEntry {
                        id: row.get(0)?,
                        ts: row.get(1)?,
                        tool: row.get(2)?,
                        connection_id: row.get(3)?,
                        space_name: row.get(4)?,
                        statement: row.get(5)?,
                        access_level: row.get(6)?,
                        status: row.get(7)?,
                        duration_ms: row.get(8)?,
                        rows_returned: row.get(9)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(entries)
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn create_test_db() -> DatabaseManager {
        let db_path =
            std::env::temp_dir().join(format!("larik_mcp_log_test_{}.db", Uuid::new_v4()));
        DatabaseManager::new(db_path).unwrap()
    }

    #[test]
    fn insert_and_list_roundtrip() {
        let manager = create_test_db();
        let entry = make_entry(
            "run_query",
            "c1",
            "My Space",
            "SELECT 1",
            "read",
            STATUS_OK,
            12,
            1,
        );
        manager.insert_mcp_log(&entry).unwrap();

        let log = manager.list_mcp_log(10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].tool, "run_query");
        assert_eq!(log[0].space_name, "My Space");
        assert_eq!(log[0].status, "ok");
        assert!(!log[0].ts.is_empty());
    }

    #[test]
    fn long_statements_are_truncated() {
        let manager = create_test_db();
        let long = "SELECT 1".repeat(5_000);
        let entry = make_entry("run_query", "c1", "s", &long, "read", STATUS_DENIED, 0, 0);
        assert!(entry.statement.chars().count() <= STATEMENT_MAX_CHARS);
        manager.insert_mcp_log(&entry).unwrap();
        let log = manager.list_mcp_log(10).unwrap();
        assert_eq!(log[0].statement.chars().count(), STATEMENT_MAX_CHARS);
    }
}
