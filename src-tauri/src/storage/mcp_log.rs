// Audit log for the embedded MCP server.
//
// Every MCP tool call — including denied ones — is recorded here so the user
// can see exactly what an agent did with their connections. `duration_ms` is
// total wall time; when an approval gate was involved, `decision_latency_ms`
// and `decided_at` separate "how long the human took" from "how long the DB
// took".

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
    /// Access level in effect for the call (`read`, `write`, or `""` for
    /// cross-space tools that have no single connection).
    pub access_level: String,
    /// `ok`, `error`, or `denied`.
    pub status: String,
    pub duration_ms: i64,
    pub rows_returned: i64,
    /// Approval-wait duration; `None` when no approval gate was involved.
    pub decision_latency_ms: Option<i64>,
    /// When the human (or auto-deny timeout) decided.
    pub decided_at: Option<String>,
}

/// Cap on how much statement text is stored per log row.
const STATEMENT_MAX_CHARS: usize = 10_000;

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
    decision_latency_ms: Option<i64>,
    decided_at: Option<String>,
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
        decision_latency_ms,
        decided_at,
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
                    access_level, status, duration_ms, rows_returned,
                    decision_latency_ms, decided_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
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
                    entry.decision_latency_ms,
                    entry.decided_at,
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
                       access_level, status, duration_ms, rows_returned,
                       decision_latency_ms, decided_at
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
                        decision_latency_ms: row.get(10)?,
                        decided_at: row.get(11)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(entries)
        })
    }
}
impl DatabaseManager {
    /// Cleanup MCP log entries older than specified retention period.
    /// Shares the `history_retention_days` setting with archived tabs.
    pub fn cleanup_old_mcp_log(&self, retention_days: i32) -> StorageResult<usize> {
        self.with_connection(|conn| {
            let affected = conn.execute(
                "DELETE FROM mcp_log
                 WHERE julianday('now') - julianday(ts) > ?",
                params![retention_days],
            )?;
            Ok(affected)
        })
    }

    /// Permanently delete ALL MCP log entries (regardless of retention period)
    pub fn purge_all_mcp_log(&self) -> StorageResult<usize> {
        self.with_connection(|conn| {
            let affected = conn.execute("DELETE FROM mcp_log", [])?;
            Ok(affected)
        })
    }
}

// ---------------------------------------------------------------------------
// Approval-request lifecycle (mcp_approvals)
// ---------------------------------------------------------------------------

impl DatabaseManager {
    /// Persist a pending approval request (called when the MCP call parks)
    pub fn insert_mcp_approval(
        &self,
        request: &crate::mcp::approvals::ApprovalRequest,
        status: &str,
    ) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO mcp_approvals (
                    id, connection_id, space_name, driver, statement, access_level, status
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    request.id,
                    request.connection_id,
                    request.space_name,
                    request.driver,
                    persist_statement(&request.statement),
                    request.access_level,
                    status,
                ],
            )?;
            Ok(())
        })
    }

    /// Transition a request out of `pending` with the decision time
    pub fn update_mcp_approval_status(
        &self,
        id: &str,
        status: &str,
        decided_at: &str,
    ) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                "UPDATE mcp_approvals SET status = ?1, decided_at = ?2 WHERE id = ?3",
                params![status, decided_at, id],
            )?;
            Ok(())
        })
    }

    /// Sweep requests left `pending` by an app crash or force-kill
    pub fn orphan_pending_mcp_approvals(&self, status: &str) -> StorageResult<usize> {
        self.with_connection(|conn| {
            let rows = conn.execute(
                "UPDATE mcp_approvals SET status = ?1 WHERE status = 'pending'",
                params![status],
            )?;
            Ok(rows)
        })
    }

    /// Recent approval requests, newest first (audit UI / tests)
    pub fn list_mcp_approvals(&self, limit: usize) -> StorageResult<Vec<McpApprovalRow>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, ts, connection_id, space_name, driver, statement,
                       access_level, status, decided_at
                FROM mcp_approvals
                ORDER BY ts DESC, id DESC
                LIMIT ?1
                "#,
            )?;

            let rows = stmt
                .query_map(params![limit], |row| {
                    Ok(McpApprovalRow {
                        id: row.get(0)?,
                        ts: row.get(1)?,
                        connection_id: row.get(2)?,
                        space_name: row.get(3)?,
                        driver: row.get(4)?,
                        statement: row.get(5)?,
                        access_level: row.get(6)?,
                        status: row.get(7)?,
                        decided_at: row.get(8)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(rows)
        })
    }
}

/// One row of `mcp_approvals`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpApprovalRow {
    pub id: String,
    pub ts: String,
    pub connection_id: String,
    pub space_name: String,
    pub driver: String,
    pub statement: String,
    pub access_level: String,
    pub status: String,
    pub decided_at: Option<String>,
}

/// Truncate statement text for persistence.
fn persist_statement(statement: &str) -> String {
    statement.chars().take(10_000).collect()
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
            None,
            None,
        );
        manager.insert_mcp_log(&entry).unwrap();

        let log = manager.list_mcp_log(10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].tool, "run_query");
        assert_eq!(log[0].space_name, "My Space");
        assert_eq!(log[0].status, "ok");
        assert!(!log[0].ts.is_empty());
        assert_eq!(log[0].decision_latency_ms, None);
    }

    #[test]
    fn decision_fields_roundtrip() {
        let manager = create_test_db();
        let entry = make_entry(
            "run_query",
            "c1",
            "My Space",
            "DELETE FROM users",
            "write",
            STATUS_DENIED,
            120_500,
            0,
            Some(120_000),
            Some("2026-10-14T10:00:00Z".to_string()),
        );
        manager.insert_mcp_log(&entry).unwrap();

        let log = manager.list_mcp_log(10).unwrap();
        assert_eq!(log[0].status, "denied");
        assert_eq!(log[0].duration_ms, 120_500);
        assert_eq!(log[0].decision_latency_ms, Some(120_000));
        assert_eq!(log[0].decided_at.as_deref(), Some("2026-10-14T10:00:00Z"));
    }

    #[test]
    fn long_statements_are_truncated() {
        let manager = create_test_db();
        let long = "SELECT 1".repeat(5_000);
        let entry = make_entry(
            "run_query",
            "c1",
            "s",
            &long,
            "read",
            STATUS_DENIED,
            0,
            0,
            None,
            None,
        );
        assert!(entry.statement.chars().count() <= STATEMENT_MAX_CHARS);
        manager.insert_mcp_log(&entry).unwrap();
        let log = manager.list_mcp_log(10).unwrap();
        assert_eq!(log[0].statement.chars().count(), STATEMENT_MAX_CHARS);
    }

    #[test]
    fn cleanup_old_mcp_log_respects_retention() {
        use rusqlite::params as sql_params;
        let manager = create_test_db();
        let entry = make_entry(
            "run_query",
            "c1",
            "s",
            "SELECT 1",
            "read",
            STATUS_OK,
            1,
            1,
            None,
            None,
        );
        manager.insert_mcp_log(&entry).unwrap();
        // Backdate the row past the retention window.
        manager
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE mcp_log SET ts = datetime('now', '-100 days') WHERE id = ?1",
                    sql_params![entry.id],
                )?;
                Ok(())
            })
            .unwrap();

        let deleted = manager.cleanup_old_mcp_log(90).unwrap();
        assert_eq!(deleted, 1);
        assert!(manager.list_mcp_log(10).unwrap().is_empty());
    }

    #[test]
    fn cleanup_old_mcp_log_keeps_fresh_rows() {
        let manager = create_test_db();
        let entry = make_entry(
            "run_query",
            "c1",
            "s",
            "SELECT 1",
            "read",
            STATUS_OK,
            1,
            1,
            None,
            None,
        );
        manager.insert_mcp_log(&entry).unwrap();

        let deleted = manager.cleanup_old_mcp_log(90).unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(manager.list_mcp_log(10).unwrap().len(), 1);

        let purged = manager.purge_all_mcp_log().unwrap();
        assert_eq!(purged, 1);
        assert!(manager.list_mcp_log(10).unwrap().is_empty());
    }
}
