//! Shared MCP policy records, response size-budget trimmer, and named-pipe wire protocol
//! shared between the `larik-sql-studio` desktop studio and the `larik-mcp` stdio server binary.

use std::time::Duration;

use larik_data::ConnectionProfile;
use serde::{Deserialize, Serialize};

/// Hard ceiling on statement length accepted by `run_query` and stored in `mcp_log` / `mcp_approvals`.
pub const MCP_MAX_STATEMENT_CHARS: usize = 10_000;
/// Maximum length of the agent-provided `description` field.
pub const MCP_MAX_DESCRIPTION_CHARS: usize = 280;
/// Maximum length of the agent-provided `reference` (ticket/incident) field.
pub const MCP_MAX_REFERENCE_CHARS: usize = 64;

/// Default and maximum rows returned per result set by `run_query`.
pub const MCP_DEFAULT_ROW_LIMIT: usize = 200;
pub const MCP_MAX_ROW_LIMIT: usize = 1_000;

/// Maximum concurrent DB tool calls allowed per workspace inside one `larik-mcp` process.
pub const MCP_MAX_CONCURRENT_PER_SPACE: usize = 2;

/// Hard per-query timeout enforced by `larik-mcp` regardless of workspace connection timeout.
pub const MCP_MAX_STATEMENT_DURATION: Duration = Duration::from_secs(120);

/// Maximum UTF-8 byte size for a single `run_query` JSON payload (1 MiB).
pub const MCP_RESPONSE_MAX_BYTES: usize = 1_048_576;

pub mod mcp_log_status {
    pub const OK: &str = "ok";
    pub const ERROR: &str = "error";
    pub const DENIED: &str = "denied";
}

pub mod mcp_approval_status {
    pub const PENDING: &str = "pending";
    pub const APPROVED: &str = "approved";
    pub const DENIED: &str = "denied";
    pub const TIMEOUT: &str = "timeout";
    pub const ORPHANED: &str = "orphaned";
}

pub mod mcp_pipe {
    use std::time::Duration;

    /// Named pipe used by `larik-mcp` to talk to the running `larik-sql-studio` desktop app.
    pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\larik.sqlstudio.mcp.v1";

    /// How long `larik-mcp` waits to connect to the running studio's pipe before assuming it is closed.
    pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(900);

    /// How long the studio's interactive approval dialog stays open before auto-denying.
    pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

    /// Client-side read backstop (`APPROVAL_TIMEOUT + 20s`) in case the desktop app exits mid-prompt.
    pub const CLIENT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(140);

    /// How long `open_in_studio` waits for the studio to acknowledge opening a tab.
    pub const OPEN_REPLY_TIMEOUT: Duration = Duration::from_secs(15);

    pub const MSG_TYPE_APPROVAL: &str = "approval";
    pub const MSG_TYPE_DECISION: &str = "decision";
    pub const MSG_TYPE_OPEN: &str = "open";
    pub const MSG_TYPE_OPENED: &str = "opened";

    pub const SURFACE_APP: &str = "app";

    /// Returns the active pipe path, honoring `LARIK_MCP_PIPE` override when set (e.g. in tests).
    pub fn pipe_name() -> String {
        if let Ok(custom) = std::env::var("LARIK_MCP_PIPE") {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        DEFAULT_PIPE_NAME.to_string()
    }
}

/// Per-workspace access mode exposed to AI agents. Always fails closed to `Off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpAccess {
    #[default]
    Off,
    Read,
    Write,
}

impl McpAccess {
    pub const ALL: [McpAccess; 3] = [McpAccess::Off, McpAccess::Read, McpAccess::Write];

    /// Fail-closed parser: any unknown, empty, or missing string maps to `McpAccess::Off`.
    pub fn parse_fail_closed(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            Some(s) if s.eq_ignore_ascii_case("read") => Self::Read,
            Some(s) if s.eq_ignore_ascii_case("write") => Self::Write,
            _ => Self::Off,
        }
    }

    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Read => "read",
            Self::Write => "write",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Read => "Read only",
            Self::Write => "Read and write",
        }
    }
}

/// Summary of one workspace's MCP exposure (non-secret profile only).
#[derive(Debug, Clone)]
pub struct McpSpaceRecord {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    pub access: McpAccess,
    pub has_connection: bool,
    pub visible_profile: ConnectionProfile,
}

/// One persisted row in `mcp_log`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpLogRecord {
    pub id: i64,
    /// SQLite UTC timestamp (`YYYY-MM-DD HH:MM:SS`).
    pub recorded_at: String,
    pub tool: String,
    pub space_id: Option<String>,
    pub space_name: Option<String>,
    pub provider_id: Option<String>,
    pub access_level: String,
    pub statement: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub duration_ms: i64,
    pub rows_returned: i64,
    pub decision_surface: Option<String>,
}

/// One persisted row in `mcp_approvals`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpApprovalRecord {
    pub id: String,
    pub created_at: String,
    pub space_id: String,
    pub space_name: Option<String>,
    pub statement: String,
    pub status: String,
    pub decided_at: Option<String>,
    pub surface: Option<String>,
}

/// Wire message sent by `larik-mcp` to `larik-sql-studio` asking the user to approve a write query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpApprovalRequest {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub id: String,
    pub space_id: String,
    pub space_name: String,
    pub provider_id: String,
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

impl McpApprovalRequest {
    pub fn new(
        id: impl Into<String>,
        space_id: impl Into<String>,
        space_name: impl Into<String>,
        provider_id: impl Into<String>,
        statement: impl Into<String>,
        description: Option<String>,
        reference: Option<String>,
    ) -> Self {
        Self {
            msg_type: mcp_pipe::MSG_TYPE_APPROVAL.to_string(),
            id: id.into(),
            space_id: space_id.into(),
            space_name: space_name.into(),
            provider_id: provider_id.into(),
            statement: truncate_chars(&statement.into(), MCP_MAX_STATEMENT_CHARS),
            description: description
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|s| truncate_chars(&s, MCP_MAX_DESCRIPTION_CHARS)),
            reference: reference
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|s| truncate_chars(&s, MCP_MAX_REFERENCE_CHARS)),
        }
    }
}

/// Wire reply sent by `larik-sql-studio` back to `larik-mcp` for an approval prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpApprovalDecision {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub id: String,
    pub approved: bool,
    pub surface: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl McpApprovalDecision {
    pub fn approved(id: impl Into<String>) -> Self {
        Self {
            msg_type: mcp_pipe::MSG_TYPE_DECISION.to_string(),
            id: id.into(),
            approved: true,
            surface: mcp_pipe::SURFACE_APP.to_string(),
            status: mcp_approval_status::APPROVED.to_string(),
            reason: None,
        }
    }

    pub fn denied(
        id: impl Into<String>,
        status: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            msg_type: mcp_pipe::MSG_TYPE_DECISION.to_string(),
            id: id.into(),
            approved: false,
            surface: mcp_pipe::SURFACE_APP.to_string(),
            status: status.into(),
            reason: Some(reason.into()),
        }
    }
}

/// Wire message sent by `larik-mcp` to `larik-sql-studio` for `open_in_studio`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOpenRequest {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub id: String,
    pub space_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_name: Option<String>,
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
}

impl McpOpenRequest {
    pub fn new(
        id: impl Into<String>,
        space_id: impl Into<String>,
        space_name: Option<String>,
        statement: impl Into<String>,
        database: Option<String>,
    ) -> Self {
        Self {
            msg_type: mcp_pipe::MSG_TYPE_OPEN.to_string(),
            id: id.into(),
            space_id: space_id.into(),
            space_name,
            statement: truncate_chars(&statement.into(), MCP_MAX_STATEMENT_CHARS),
            database: database
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        }
    }
}

/// Wire reply sent by `larik-sql-studio` back to `larik-mcp` for `open_in_studio`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOpenResponse {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl McpOpenResponse {
    pub fn ok(id: impl Into<String>) -> Self {
        Self {
            msg_type: mcp_pipe::MSG_TYPE_OPENED.to_string(),
            id: id.into(),
            ok: true,
            reason: None,
        }
    }

    pub fn err(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            msg_type: mcp_pipe::MSG_TYPE_OPENED.to_string(),
            id: id.into(),
            ok: false,
            reason: Some(reason.into()),
        }
    }
}

/// Discriminated union parsed by the studio pipe server from an incoming JSON line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpPipeIncoming {
    Approval(McpApprovalRequest),
    Open(McpOpenRequest),
}

impl McpPipeIncoming {
    pub fn parse_line(line: &str) -> anyhow::Result<Self> {
        let value: serde_json::Value = serde_json::from_str(line)?;
        let msg_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or(mcp_pipe::MSG_TYPE_APPROVAL);
        if msg_type.eq_ignore_ascii_case(mcp_pipe::MSG_TYPE_OPEN) {
            Ok(Self::Open(serde_json::from_value(value)?))
        } else {
            Ok(Self::Approval(serde_json::from_value(value)?))
        }
    }
}

/// One column in a `run_query` result set payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpColumnPayload {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}

/// One result set in a `run_query` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResultSetPayload {
    pub columns: Vec<McpColumnPayload>,
    pub rows: Vec<Vec<Option<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records_affected: Option<i64>,
    pub truncated: bool,
    pub message: String,
    pub elapsed_ms: u64,
}

/// Complete JSON response returned by `run_query`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpQueryPayload {
    pub connection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub row_limit: usize,
    pub result_sets: Vec<McpResultSetPayload>,
    pub messages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_query: Option<String>,
    pub truncated_for_size: bool,
}

/// Serializes `payload` under `max_bytes`, iteratively dropping rows from the largest result set
/// in 10% chunks (up to 60 attempts) — matching WinUI `McpResponseBudget.Trim`.
pub fn trim_response_budget(payload: &mut McpQueryPayload, max_bytes: usize) -> String {
    let mut json = serde_json::to_string(payload).unwrap_or_default();
    if json.len() <= max_bytes {
        return json;
    }

    for _ in 0..60 {
        // Pick the result set with the most remaining rows.
        let largest_ix = payload
            .result_sets
            .iter()
            .enumerate()
            .filter(|(_, rs)| !rs.rows.is_empty())
            .max_by_key(|(_, rs)| rs.rows.len())
            .map(|(ix, _)| ix);

        let Some(ix) = largest_ix else {
            break;
        };

        let rs = &mut payload.result_sets[ix];
        let drop_count = (rs.rows.len() / 10).max(1);
        let new_len = rs.rows.len().saturating_sub(drop_count);
        rs.rows.truncate(new_len);
        rs.truncated = true;
        payload.truncated_for_size = true;

        json = serde_json::to_string(payload).unwrap_or_default();
        if json.len() <= max_bytes {
            return json;
        }
    }

    if json.len() > max_bytes {
        let fallback = serde_json::json!({
            "truncated": true,
            "message": format!(
                "The result exceeded {} bytes even after dropping rows. Narrow the query with a smaller column list or row_limit.",
                max_bytes
            )
        });
        return serde_json::to_string(&fallback).unwrap_or_default();
    }

    json
}

pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_levels_fail_closed_and_round_trip() {
        assert_eq!(McpAccess::parse_fail_closed(None), McpAccess::Off);
        assert_eq!(McpAccess::parse_fail_closed(Some("")), McpAccess::Off);
        assert_eq!(McpAccess::parse_fail_closed(Some("admin")), McpAccess::Off);
        assert_eq!(McpAccess::parse_fail_closed(Some("read_write")), McpAccess::Off);

        for level in McpAccess::ALL {
            assert_eq!(
                McpAccess::parse_fail_closed(Some(level.as_storage())),
                level
            );
        }
    }

    #[test]
    fn pipe_protocol_round_trips_requests_and_decisions() {
        let req = McpApprovalRequest::new(
            "abc123",
            "space-1",
            "Prod",
            "mssql",
            "UPDATE dbo.orders SET status = 'shipped' WHERE id = 1",
            Some("Mark order shipped".into()),
            Some("FIX-101".into()),
        );
        let line = serde_json::to_string(&req).unwrap();
        let parsed = McpPipeIncoming::parse_line(&line).unwrap();
        assert_eq!(parsed, McpPipeIncoming::Approval(req));

        let open = McpOpenRequest::new(
            "open-1",
            "space-1",
            Some("Prod".into()),
            "SELECT 1",
            Some("master".into()),
        );
        let open_line = serde_json::to_string(&open).unwrap();
        let parsed_open = McpPipeIncoming::parse_line(&open_line).unwrap();
        assert_eq!(parsed_open, McpPipeIncoming::Open(open));

        let denied = McpApprovalDecision::denied(
            "abc123",
            mcp_approval_status::TIMEOUT,
            "Timed out after two minutes.",
        );
        let dec_json = serde_json::to_string(&denied).unwrap();
        let dec_back: McpApprovalDecision = serde_json::from_str(&dec_json).unwrap();
        assert_eq!(dec_back, denied);
        assert!(!dec_back.approved);
        assert_eq!(dec_back.surface, mcp_pipe::SURFACE_APP);
        assert!(mcp_pipe::CONNECT_TIMEOUT < mcp_pipe::APPROVAL_TIMEOUT);
        assert_eq!(mcp_pipe::APPROVAL_TIMEOUT, Duration::from_secs(120));
    }

    #[test]
    fn response_budget_trims_largest_result_set_first() {
        let big_cell = "x".repeat(400);
        let mut payload = McpQueryPayload {
            connection: "c1".into(),
            database: Some("db".into()),
            row_limit: 200,
            result_sets: vec![
                McpResultSetPayload {
                    columns: vec![McpColumnPayload {
                        name: "small".into(),
                        r#type: Some("int".into()),
                    }],
                    rows: vec![vec![Some("1".into())], vec![Some("2".into())]],
                    records_affected: Some(2),
                    truncated: false,
                    message: "2 rows".into(),
                    elapsed_ms: 1,
                },
                McpResultSetPayload {
                    columns: vec![McpColumnPayload {
                        name: "big".into(),
                        r#type: Some("nvarchar".into()),
                    }],
                    rows: (0..40).map(|_| vec![Some(big_cell.clone())]).collect(),
                    records_affected: Some(40),
                    truncated: false,
                    message: "40 rows".into(),
                    elapsed_ms: 2,
                },
            ],
            messages: vec![],
            generated_query: None,
            truncated_for_size: false,
        };

        let json = trim_response_budget(&mut payload, 5_000);
        assert!(json.len() <= 5_000);
        assert!(payload.truncated_for_size);
        assert!(!payload.result_sets[0].truncated);
        assert_eq!(payload.result_sets[0].rows.len(), 2);
        assert!(payload.result_sets[1].truncated);
        assert!(payload.result_sets[1].rows.len() < 40);
        assert!(!payload.result_sets[1].rows.is_empty());
    }
}
