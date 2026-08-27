// Query Execution Engine (T017)
// Handles non-blocking query execution with result streaming

use crate::db::connection::{ConnectionError, MssqlConnectionManager};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tiberius::{Column, ColumnType, Row, ToSql};
use tiberius::numeric::Numeric;
use tokio::sync::{RwLock, oneshot};
use uuid::Uuid;

// Logging macros using println for simplicity (no trailing semicolon for use in match arms)
macro_rules! log_info {
    ($($arg:tt)*) => {{ println!("[INFO] {}", format!($($arg)*)) }};
}
macro_rules! log_warn {
    ($($arg:tt)*) => {{ println!("[WARN] {}", format!($($arg)*)) }};
}

/// Represents a single cell value in the result set
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CellValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    DateTime(String),
    Binary(Vec<u8>),
}

impl CellValue {
    /// Convert a tiberius column value to CellValue
    pub fn from_row(row: &Row, idx: usize, col_type: &ColumnType) -> Self {
        // Check for NULL first
        if row.try_get::<&str, _>(idx).ok().flatten().is_none() 
            && row.try_get::<i32, _>(idx).ok().flatten().is_none()
            && row.try_get::<bool, _>(idx).ok().flatten().is_none() 
        {
            // Try to detect if it's actually NULL
            match col_type {
                ColumnType::Null => return CellValue::Null,
                _ => {}
            }
        }

        match col_type {
            ColumnType::Null => CellValue::Null,
            
            // Integer types
            ColumnType::Int1 => row.try_get::<u8, _>(idx)
                .ok().flatten()
                .map(|v| CellValue::Int(v as i64))
                .unwrap_or(CellValue::Null),
            ColumnType::Int2 => row.try_get::<i16, _>(idx)
                .ok().flatten()
                .map(|v| CellValue::Int(v as i64))
                .unwrap_or(CellValue::Null),
            ColumnType::Int4 => row.try_get::<i32, _>(idx)
                .ok().flatten()
                .map(|v| CellValue::Int(v as i64))
                .unwrap_or(CellValue::Null),
            ColumnType::Int8 => row.try_get::<i64, _>(idx)
                .ok().flatten()
                .map(CellValue::Int)
                .unwrap_or(CellValue::Null),
            ColumnType::Intn => row.try_get::<i64, _>(idx)
                .ok().flatten()
                .map(CellValue::Int)
                .or_else(|| row.try_get::<i32, _>(idx).ok().flatten().map(|v| CellValue::Int(v as i64)))
                .unwrap_or(CellValue::Null),
            
            // Float types
            ColumnType::Float4 => row.try_get::<f32, _>(idx)
                .ok().flatten()
                .map(|v| CellValue::Float(v as f64))
                .unwrap_or(CellValue::Null),
            ColumnType::Float8 => row.try_get::<f64, _>(idx)
                .ok().flatten()
                .map(CellValue::Float)
                .unwrap_or(CellValue::Null),
            ColumnType::Floatn => row.try_get::<f64, _>(idx)
                .ok().flatten()
                .map(CellValue::Float)
                .or_else(|| row.try_get::<f32, _>(idx).ok().flatten().map(|v| CellValue::Float(v as f64)))
                .unwrap_or(CellValue::Null),
            
            // Decimal/Numeric types - use Numeric type from Tiberius
            ColumnType::Decimaln | ColumnType::Numericn => {
                row.try_get::<Numeric, _>(idx)
                    .ok().flatten()
                    .map(|n| {
                        // Convert to f64 using From trait
                        CellValue::Float(f64::from(n))
                    })
                    .unwrap_or(CellValue::Null)
            }
            
            // Money types - Tiberius returns these as f64
            ColumnType::Money | ColumnType::Money4 => {
                row.try_get::<f64, _>(idx)
                    .ok().flatten()
                    .map(CellValue::Float)
                    .unwrap_or(CellValue::Null)
            }
            
            // Boolean
            ColumnType::Bit | ColumnType::Bitn => row.try_get::<bool, _>(idx)
                .ok().flatten()
                .map(CellValue::Bool)
                .unwrap_or(CellValue::Null),
            
            // String types
            ColumnType::BigVarChar | ColumnType::BigChar | ColumnType::NVarchar | ColumnType::NChar |
            ColumnType::Text | ColumnType::NText => {
                row.try_get::<&str, _>(idx)
                    .ok().flatten()
                    .map(|s| CellValue::String(s.to_string()))
                    .unwrap_or(CellValue::Null)
            }
            
            // Date/Time types
            ColumnType::Datetime | ColumnType::Datetime2 | ColumnType::Datetimen => {
                row.try_get::<NaiveDateTime, _>(idx)
                    .ok().flatten()
                    .map(|dt| CellValue::DateTime(dt.to_string()))
                    .unwrap_or(CellValue::Null)
            }
            ColumnType::Datetime4 => {
                row.try_get::<NaiveDateTime, _>(idx)
                    .ok().flatten()
                    .map(|dt| CellValue::DateTime(dt.to_string()))
                    .unwrap_or(CellValue::Null)
            }
            ColumnType::DatetimeOffsetn => {
                row.try_get::<DateTime<Utc>, _>(idx)
                    .ok().flatten()
                    .map(|dt| CellValue::DateTime(dt.to_rfc3339()))
                    .unwrap_or(CellValue::Null)
            }
            ColumnType::Daten => {
                row.try_get::<NaiveDate, _>(idx)
                    .ok().flatten()
                    .map(|d| CellValue::DateTime(d.to_string()))
                    .unwrap_or(CellValue::Null)
            }
            ColumnType::Timen => {
                row.try_get::<NaiveTime, _>(idx)
                    .ok().flatten()
                    .map(|t| CellValue::DateTime(t.to_string()))
                    .unwrap_or(CellValue::Null)
            }
            
            // Binary types
            ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image => {
                row.try_get::<&[u8], _>(idx)
                    .ok().flatten()
                    .map(|b| CellValue::Binary(b.to_vec()))
                    .unwrap_or(CellValue::Null)
            }
            
            // GUID
            ColumnType::Guid => {
                row.try_get::<tiberius::Uuid, _>(idx)
                    .ok().flatten()
                    .map(|u| CellValue::String(u.to_string()))
                    .unwrap_or(CellValue::Null)
            }
            
            // XML - Tiberius returns XML as XmlData type
            ColumnType::Xml => {
                row.try_get::<&tiberius::xml::XmlData, _>(idx)
                    .ok().flatten()
                    .map(|xml| {
                        let s = xml.to_owned().into_string();
                        CellValue::String(s)
                    })
                    .unwrap_or(CellValue::Null)
            }
            
            // Default: try as string
            _ => {
                row.try_get::<&str, _>(idx)
                    .ok().flatten()
                    .map(|s| CellValue::String(s.to_string()))
                    .unwrap_or(CellValue::Null)
            }
        }
    }
}

/// Column metadata for the result set
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

impl From<&Column> for ColumnInfo {
    fn from(col: &Column) -> Self {
        Self {
            name: col.name().to_string(),
            data_type: format_sql_data_type(&col.column_type()),
            nullable: true, // Tiberius doesn't easily expose nullability
        }
    }
}

/// Convert Tiberius ColumnType to user-friendly SQL data type string
fn format_sql_data_type(col_type: &ColumnType) -> String {
    match col_type {
        // Integer types
        ColumnType::Int1 => "tinyint".to_string(),
        ColumnType::Int2 => "smallint".to_string(),
        ColumnType::Int4 => "int".to_string(),
        ColumnType::Int8 => "bigint".to_string(),
        ColumnType::Intn => "int".to_string(),
        
        // Float types
        ColumnType::Float4 => "real".to_string(),
        ColumnType::Float8 => "float".to_string(),
        ColumnType::Floatn => "float".to_string(),
        
        // Decimal/Numeric types
        ColumnType::Decimaln => "decimal".to_string(),
        ColumnType::Numericn => "numeric".to_string(),
        ColumnType::Money => "money".to_string(),
        ColumnType::Money4 => "smallmoney".to_string(),
        
        // Boolean types
        ColumnType::Bit => "bit".to_string(),
        ColumnType::Bitn => "bit".to_string(),
        
        // String types
        ColumnType::BigVarChar => "varchar(max)".to_string(),
        ColumnType::BigChar => "char(max)".to_string(),
        ColumnType::NVarchar => "nvarchar".to_string(),
        ColumnType::NChar => "nchar".to_string(),
        ColumnType::Text => "text".to_string(),
        ColumnType::NText => "ntext".to_string(),
        
        // Binary types
        ColumnType::BigVarBin => "varbinary(max)".to_string(),
        ColumnType::BigBinary => "binary(max)".to_string(),
        ColumnType::Image => "image".to_string(),
        
        // Date/Time types
        ColumnType::Datetime => "datetime".to_string(),
        ColumnType::Datetime2 => "datetime2".to_string(),
        ColumnType::Datetimen => "datetime".to_string(),
        ColumnType::Datetime4 => "smalldatetime".to_string(),
        ColumnType::Daten => "date".to_string(),
        ColumnType::Timen => "time".to_string(),
        ColumnType::DatetimeOffsetn => "datetimeoffset".to_string(),
        
        // Other types
        ColumnType::Guid => "uniqueidentifier".to_string(),
        ColumnType::Xml => "xml".to_string(),
        ColumnType::Null => "null".to_string(),
        
        // For any other types, fall back to debug format
        _ => format!("{:?}", col_type).to_lowercase(),
    }
}

/// Query result containing columns and rows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub query_id: String,
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<CellValue>>,
    pub row_count: usize,
    pub truncated: bool,
    pub limit_applied: Option<usize>,
    pub execution_time_ms: u64,
    pub error: Option<String>,
    pub is_complete: bool,
    pub is_selection: bool, // Indicates if this was executed from selected text
    pub statement_index: Option<usize>, // Index in batch execution (None for single query)
    pub statement_text: Option<String>, // The actual SQL text executed (useful for batch)
}

impl QueryResult {
    pub fn new(query_id: String) -> Self {
        Self {
            query_id,
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
            truncated: false,
            limit_applied: None,
            execution_time_ms: 0,
            error: None,
            is_complete: false,
            is_selection: false,
            statement_index: None,
            statement_text: None,
        }
    }

    pub fn with_error(query_id: String, error: String) -> Self {
        Self {
            query_id,
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
            truncated: false,
            limit_applied: None,
            execution_time_ms: 0,
            error: Some(error),
            is_complete: true,
            is_selection: false,
            statement_index: None,
            statement_text: None,
        }
    }
}

/// Status of a running query
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum QueryStatus {
    Pending,
    Running,
    Completed,
    Cancelled,
    Error,
}

/// Information about a running query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryInfo {
    pub query_id: String,
    pub connection_id: String,
    pub query: String,
    pub status: QueryStatus,
    pub started_at: String,
    pub rows_fetched: usize,
}

/// Query execution engine
pub struct QueryEngine {
    connection_manager: Arc<MssqlConnectionManager>,
    /// Track running query cancellation senders - sending cancels the query
    cancel_senders: RwLock<HashMap<String, oneshot::Sender<()>>>,
    /// Query info for status checking
    query_info: RwLock<HashMap<String, QueryInfo>>,
}

/// Auto-wrap procedure calls without EXEC keyword
/// If a statement looks like a bare procedure name (e.g., "sp_who2", "dbo.sp_who2"),
/// wrap it with EXEC to allow execution without explicit EXEC keyword (like SSMS/DBeaver).
fn auto_wrap_procedure(stmt: String) -> String {
    let trimmed = stmt.trim();
    
    // If already wrapped with EXEC, return as-is
    if trimmed.to_uppercase().starts_with("EXEC") || trimmed.to_uppercase().starts_with("EXECUTE") {
        return stmt;
    }
    
    // If it starts with any of these keywords, it's not a bare procedure call
    let reserved_keywords = [
        "SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP",
        "DECLARE", "SET", "IF", "BEGIN", "END", "WHILE", "FOR", "MERGE",
        "WITH", "UNION", "USE", "PRINT", "RETURN", "CAST", "CASE",
    ];
    
    let first_word = trimmed.split_whitespace().next().unwrap_or("").to_uppercase();
    
    // If it starts with a reserved keyword, it's not a bare procedure
    if reserved_keywords.iter().any(|&kw| first_word == kw) {
        return stmt;
    }
    
    // Check if it looks like a procedure name (identifier or schema.identifier)
    // Pattern: [a-zA-Z_][a-zA-Z0-9_]* optionally prefixed with [schema.]
    // and potentially followed by parentheses with parameters
    let is_likely_procedure = trimmed.chars().next()
        .map(|c| c.is_alphabetic() || c == '_' || c == '[')
        .unwrap_or(false);
    
    if is_likely_procedure {
        // Check if it contains spaces (likely a procedure with parameters)
        // or if it's a simple identifier/schema.identifier pattern
        let has_parentheses = trimmed.contains('(');
        let trimmed_upper = trimmed.to_uppercase();
        
        // If it has WITH, ORDER BY, GROUP BY, WHERE, FROM, or other SQL keywords, it's a query
        if trimmed_upper.contains(" WITH") || trimmed_upper.contains(" ORDER BY") ||
           trimmed_upper.contains(" GROUP BY") || trimmed_upper.contains(" WHERE") ||
           trimmed_upper.contains(" FROM") || trimmed_upper.contains(" JOIN") {
            return stmt;
        }
        
        // If it looks like a simple procedure call, wrap it with EXEC
        if !has_parentheses || trimmed_upper.starts_with("SP_") || 
           trimmed_upper.starts_with("DBO.SP_") || trimmed_upper.contains(".") {
            return format!("EXEC {}", trimmed);
        }
    }
    
    stmt
}

/// Parse SQL text into individual statements.
///
/// Splitting strategy:
/// 1. Split on the `GO` batch separator (line-level, case-insensitive, only
///    when not inside a string or block comment). `GO` is a true batch boundary
///    in T-SQL — variables and temp tables do not survive across it.
/// 2. Within each `GO` batch, decide whether to also split on semicolons:
///    - If the batch needs scope preservation (contains `DECLARE`, a
///      statement-start `SET`, or `#`/`##` temp-table references outside
///      strings/comments/brackets), keep the WHOLE batch as a single statement
///      so it is sent to the server in one TDS request. Splitting such a batch
///      would either lose variable/temp-table scope or produce spurious empty
///      result tabs (one per non-rowset statement).
///    - Otherwise split on `;` so independent statements each get their own
///      result tab.
///
/// Auto-wrapping: if a statement is just a procedure name (e.g., "sp_who2"),
/// it's automatically wrapped with EXEC to allow execution without an explicit
/// EXEC keyword (like SSMS/DBeaver).
fn parse_sql_statements(sql: &str) -> Vec<String> {
    let trimmed_sql = sql.trim();
    if trimmed_sql.is_empty() {
        return Vec::new();
    }

    // Phase 1: split into batches on GO.
    let go_batches = split_on_go(trimmed_sql);

    // Phase 2: per batch, either keep whole (scope-sensitive) or split on `;`.
    let mut statements = Vec::new();
    for batch in go_batches {
        let batch = batch.trim();
        if batch.is_empty() {
            continue;
        }

        if batch_needs_scope_preservation(batch) {
            statements.push(auto_wrap_procedure(batch.to_string()));
        } else {
            for s in split_on_semicolon(batch) {
                let stmt = s.trim();
                if !stmt.is_empty() && stmt != ";" {
                    statements.push(auto_wrap_procedure(s));
                }
            }
        }
    }

    // If nothing parsed, return the original as a single statement.
    if statements.is_empty() && !trimmed_sql.is_empty() {
        statements.push(auto_wrap_procedure(trimmed_sql.to_string()));
    }

    statements
}

/// Split SQL on the `GO` batch separator (`GO` on its own line,
/// case-insensitive), ignoring `GO` inside strings or block comments.
fn split_on_go(sql: &str) -> Vec<String> {
    let mut batches = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut in_block_comment = false;
    let mut in_line_comment = false;
    let mut prev_char = '\0';

    for line in sql.lines() {
        let trimmed = line.trim();

        // GO is a separator only when not inside a string/block comment/line
        // comment at the start of the line.
        let go_separates = !in_string
            && !in_block_comment
            && !in_line_comment
            && trimmed.eq_ignore_ascii_case("go");

        if go_separates {
            let batch = current.trim().to_string();
            if !batch.is_empty() {
                batches.push(batch);
            }
            current.clear();
            // Reset state for the next batch — GO cannot be mid-string/comment.
            in_string = false;
            in_block_comment = false;
            in_line_comment = false;
            prev_char = '\0';
            continue;
        }

        // Scan the line to keep string/comment state accurate.
        for ch in line.chars() {
            if ch == '\'' && prev_char != '\\' && !in_block_comment && !in_line_comment {
                in_string = !in_string;
            }
            if !in_string {
                if ch == '-' && prev_char == '-' && !in_block_comment {
                    in_line_comment = true;
                } else if ch == '*' && prev_char == '/' && !in_line_comment {
                    in_block_comment = true;
                } else if ch == '/' && prev_char == '*' && in_block_comment {
                    in_block_comment = false;
                }
            }
            prev_char = ch;
        }

        in_line_comment = false;
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    let final_batch = current.trim().to_string();
    if !final_batch.is_empty() {
        batches.push(final_batch);
    }

    if batches.is_empty() && !sql.trim().is_empty() {
        batches.push(sql.trim().to_string());
    }

    batches
}

/// Split a SQL batch on semicolons that are outside strings, line comments, and
/// block comments. Returns each non-empty statement (terminator retained on the
/// statement text to match historical behavior).
fn split_on_semicolon(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut in_block_comment = false;
    let mut in_line_comment = false;
    let mut prev_char = '\0';

    for line in sql.lines() {
        for ch in line.chars() {
            if ch == '\'' && prev_char != '\\' && !in_block_comment && !in_line_comment {
                in_string = !in_string;
            }
            if !in_string {
                if ch == '-' && prev_char == '-' && !in_block_comment {
                    in_line_comment = true;
                } else if ch == '*' && prev_char == '/' && !in_line_comment {
                    in_block_comment = true;
                } else if ch == '/' && prev_char == '*' && in_block_comment {
                    in_block_comment = false;
                }
            }

            if ch == ';' && !in_string && !in_block_comment && !in_line_comment {
                current.push(ch);
                let stmt = current.trim().to_string();
                if !stmt.is_empty() && stmt != ";" {
                    statements.push(stmt);
                }
                current.clear();
                prev_char = ch;
                continue;
            }

            current.push(ch);
            prev_char = ch;
        }
        in_line_comment = false;
        current.push('\n');
    }

    let final_stmt = current.trim().to_string();
    if !final_stmt.is_empty() && final_stmt != ";" {
        statements.push(final_stmt);
    }

    statements
}

/// Detect whether a SQL batch must be sent to the server as a single statement
/// to preserve scope. Returns true when the batch (outside strings, comments,
/// and `[...]` bracketed identifiers) contains any of:
///
/// - `DECLARE` at the start of a statement (after `;` or at batch start)
/// - `SET` at the start of a statement (covers session SETs like NOCOUNT as
///   well as variable assignments — splitting those off otherwise yields
///   spurious empty result tabs)
/// - A `#` or `##` temp-table reference (`#` followed by an identifier char)
///
/// Each signals that the batch carries state (variable or temp-table scope, or
/// a session SET that should not produce its own empty result tab) across `;`
/// boundaries within the batch.
fn batch_needs_scope_preservation(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0usize;
    let mut in_string = false;
    let mut in_block_comment = false;
    let mut in_line_comment = false;
    let mut statement_start = true;

    while i < bytes.len() {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied();

        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if b == b'*' && next == Some(b'/') {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_string {
            if b == b'\'' {
                if next == Some(b'\'') {
                    i += 2;
                } else {
                    in_string = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        // Not in string/comment — detect comment starts.
        if b == b'-' && next == Some(b'-') {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if b == b'/' && next == Some(b'*') {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if b == b'\'' {
            in_string = true;
            i += 1;
            continue;
        }

        // Semicolons end a statement; whitespace doesn't change statement_start.
        if b == b';' {
            statement_start = true;
            i += 1;
            continue;
        }
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // `[...]` bracketed identifiers — skip contents (a `#` inside brackets
        // is part of an identifier name, e.g. `[#count]` column alias, not a
        // temp table reference).
        if b == b'[' {
            i += 1;
            while i < bytes.len() && bytes[i] != b']' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // consume `]`
            }
            statement_start = false;
            continue;
        }

        // Temp-table reference: `#` or `##` followed by an identifier char,
        // outside strings/comments/brackets.
        if b == b'#' {
            let after_hash = if next == Some(b'#') {
                bytes.get(i + 2).copied()
            } else {
                next
            };
            if let Some(c) = after_hash {
                if c.is_ascii_alphanumeric() || c == b'_' {
                    return true;
                }
            }
            statement_start = false;
            i += 1;
            continue;
        }

        // Identifier-start at a statement boundary — read the keyword.
        if statement_start && (b.is_ascii_alphabetic() || b == b'_') {
            let start = i;
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let upper = sql[start..i].to_uppercase();
            if upper == "DECLARE" {
                return true;
            }
            if upper == "SET" {
                // Any statement-start SET: covers session SETs (NOCOUNT,
                // ROWCOUNT) that would otherwise produce spurious empty
                // tabs, and SET @var assignments.
                return true;
            }
            statement_start = false;
            continue;
        }

        // Any other non-whitespace token ends the statement-start window.
        statement_start = false;
        i += 1;
    }

    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatementKind {
    Dml,
    Other,
}

fn starts_with_keyword(input: &str, keyword: &str) -> bool {
    if !input.starts_with(keyword) {
        return false;
    }

    match input.as_bytes().get(keyword.len()) {
        None => true,
        Some(next) => !next.is_ascii_alphanumeric() && *next != b'_',
    }
}

fn infer_statement_kind(sql: &str) -> StatementKind {
    let mut last_kind = StatementKind::Other;

    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut statement_start = true;

    let bytes = sql.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied();

        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }

        if in_block_comment {
            if b == b'*' && next == Some(b'/') {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if in_string {
            if b == b'\'' {
                if next == Some(b'\'') {
                    i += 2;
                } else {
                    in_string = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        if b == b'-' && next == Some(b'-') {
            in_line_comment = true;
            i += 2;
            continue;
        }

        if b == b'/' && next == Some(b'*') {
            in_block_comment = true;
            i += 2;
            continue;
        }

        if b == b'\'' {
            in_string = true;
            i += 1;
            continue;
        }

        if b == b';' {
            statement_start = true;
            i += 1;
            continue;
        }

        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if statement_start {
            if b.is_ascii_alphabetic() || b == b'_' {
                let start = i;
                i += 1;
                while i < bytes.len() {
                    let c = bytes[i];
                    if c.is_ascii_alphanumeric() || c == b'_' {
                        i += 1;
                    } else {
                        break;
                    }
                }

                let upper = sql[start..i].to_uppercase();

                if starts_with_keyword(&upper, "UPDATE")
                    || starts_with_keyword(&upper, "INSERT")
                    || starts_with_keyword(&upper, "DELETE")
                    || starts_with_keyword(&upper, "MERGE")
                {
                    last_kind = StatementKind::Dml;
                } else if starts_with_keyword(&upper, "SELECT")
                    || starts_with_keyword(&upper, "EXEC")
                    || starts_with_keyword(&upper, "EXECUTE")
                    || starts_with_keyword(&upper, "CREATE")
                    || starts_with_keyword(&upper, "ALTER")
                    || starts_with_keyword(&upper, "DROP")
                    || starts_with_keyword(&upper, "TRUNCATE")
                    || starts_with_keyword(&upper, "WITH")
                {
                    last_kind = StatementKind::Other;
                }
            } else {
                i += 1;
            }

            statement_start = false;
            continue;
        }

        i += 1;
    }

    last_kind
}

#[cfg(test)]
mod tests {
    use super::{infer_statement_kind, parse_sql_statements, batch_needs_scope_preservation, StatementKind};

    #[test]
    fn infer_kind_for_declare_then_update_is_dml() {
        let sql = "\
DECLARE @ExistingChildId int = (SELECT Id FROM ProductQuestions WHERE ShortName = 'CLMoreThan25FromUSOrCanada');
UPDATE ProductQuestion_Rule
SET EffectiveFrom = '2026-05-01'
WHERE ProductQuestionId = 2856;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Dml);
    }

    #[test]
    fn infer_kind_for_update_with_subquery_select_is_dml() {
        let sql = "\
UPDATE ProductQuestion_Rule
SET EffectiveFrom = (SELECT MAX(EffectiveFrom) FROM ProductQuestion_Rule)
WHERE ProductQuestionId = 2856;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Dml);
    }

    #[test]
    fn infer_kind_ignores_comments_and_uses_last_executable_statement() {
        let sql = "\
/* UPDATE ProductQuestion_Rule SET EffectiveFrom = '1900-01-01' */
-- DELETE FROM ProductQuestion_Rule WHERE ProductQuestionId = 2856
DECLARE @note nvarchar(100) = 'comment mentions UPDATE';
UPDATE ProductQuestion_Rule SET EffectiveFrom = '2026-05-01' WHERE ProductQuestionId = 2856;
SELECT EffectiveFrom FROM ProductQuestion_Rule WHERE ProductQuestionId = 2856;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Other);
    }

    #[test]
    fn infer_kind_ignores_keywords_inside_string_literals() {
        let sql = "\
SELECT 'UPDATE ProductQuestion_Rule SET EffectiveFrom = ''2026-05-01''' AS script_text;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Other);
    }

    #[test]
    fn infer_kind_for_select_then_update_is_dml() {
        let sql = "\
SELECT TOP 1 Id FROM ProductQuestion_Rule;
UPDATE ProductQuestion_Rule SET EffectiveFrom = '2026-05-01' WHERE ProductQuestionId = 2856;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Dml);
    }

    // --- parse_sql_statements: scope preservation + splitting ---

    #[test]
    fn splits_independent_selects_on_semicolons() {
        let sql = "SELECT 1; SELECT 2; SELECT 3;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 3);
        assert!(stmts[0].trim().ends_with(';'));
    }

    #[test]
    fn keeps_declare_batch_together_when_declare_leads() {
        // Old behavior: starts_with("DECLARE") kept the whole input together.
        // The new logic must preserve that.
        let sql = "DECLARE @x INT = 1; SELECT @x;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 1, "DECLARE-led batch must stay as one statement");
        assert!(stmts[0].contains("DECLARE @x"));
    }

    #[test]
    fn keeps_declare_batch_together_when_declare_is_in_the_middle() {
        // Regression: previously only `starts_with DECLARE/SET` triggered
        // scope preservation, so a non-DECLARE prefix caused the batch to be
        // split at every `;`, losing variable scope and producing spurious
        // (often errored) result tabs.
        let sql = "USE [MyDb];\nDECLARE @x INT = 1;\nSELECT @x;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 1, "USE-prefixed DECLARE batch must stay as one");
        assert!(stmts[0].contains("USE [MyDb]"));
        assert!(stmts[0].contains("DECLARE @x"));
        assert!(stmts[0].contains("SELECT @x"));
    }

    #[test]
    fn keeps_set_session_statements_with_following_select() {
        // `SET NOCOUNT ON; SELECT ...` — splitting would create an empty
        // result tab for the SET plus the real SELECT tab. Keeping the batch
        // together yields only the SELECT's rowset.
        let sql = "SET NOCOUNT ON;\nSELECT * FROM Users;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn keeps_temp_table_create_then_select_together() {
        // The user's reported scenario: temp table + SELECT INTO then SELECT
        // FROM it, split on `;`, errors on the second tab if the first failed.
        let sql = "SELECT * INTO #t FROM Foo;\nSELECT * FROM #t;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 1, "temp-table batch must stay as one statement");
        assert!(stmts[0].contains("#t"));
    }

    #[test]
    fn still_splits_across_GO_within_declare_batch() {
        // GO is a real batch separator in T-SQL — variables do not survive
        // across it. Even a DECLARE-containing batch splits on GO.
        let sql = "DECLARE @x INT = 1;\nSELECT @x;\nGO\nSELECT @x;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("DECLARE @x"));
        assert!(!stmts[1].contains("DECLARE"));
    }

    #[test]
    fn does_not_split_on_semicolons_inside_strings() {
        let sql = "SELECT 'a;b' AS x; SELECT 'c' AS y;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("'a;b'"));
    }

    #[test]
    fn does_not_split_on_GO_inside_block_comment() {
        let sql = "/* GO */\nSELECT 1;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 1);
    }

    // --- batch_needs_scope_preservation: detection edge cases ---

    #[test]
    fn scope_detects_declare_anywhere_outside_comments() {
        assert!(batch_needs_scope_preservation("SELECT 1; DECLARE @x INT; SELECT @x;"));
    }

    #[test]
    fn scope_ignores_declare_inside_string_literal() {
        assert!(!batch_needs_scope_preservation("SELECT 'DECLARE @x'; SELECT 1;"));
    }

    #[test]
    fn scope_detects_hash_temp_table_reference() {
        assert!(batch_needs_scope_preservation("SELECT * INTO #t FROM Foo; SELECT * FROM #t;"));
        assert!(batch_needs_scope_preservation("SELECT * FROM ##global;"));
    }

    #[test]
    fn scope_ignores_hash_inside_bracketed_identifier() {
        // `[#count]` is a column alias, not a temp table.
        assert!(!batch_needs_scope_preservation("SELECT [x] AS [#count] FROM T; SELECT 1;"));
    }

    #[test]
    fn scope_ignores_hash_inside_line_comment() {
        assert!(!batch_needs_scope_preservation("-- SELECT * FROM #temp\nSELECT 1;"));
    }

    #[test]
    fn scope_detects_set_at_statement_start_only() {
        assert!(batch_needs_scope_preservation("SET @x = 1; SELECT @x;"));
        assert!(batch_needs_scope_preservation("SET NOCOUNT ON; SELECT 1;"));
        // SET inside UPDATE...SET is NOT a statement-start SET.
        assert!(!batch_needs_scope_preservation("UPDATE T SET x = 1;"));
    }

    #[test]
    fn scope_returns_false_for_plain_independent_selects() {
        assert!(!batch_needs_scope_preservation("SELECT 1; SELECT 2;"));
    }
}

impl QueryEngine {
    pub fn new(connection_manager: Arc<MssqlConnectionManager>) -> Self {
        Self {
            connection_manager,
            cancel_senders: RwLock::new(HashMap::new()),
            query_info: RwLock::new(HashMap::new()),
        }
    }

    /// Execute a query (single or batch) and return results
    /// If the query contains multiple statements (separated by GO or semicolons),
    /// executes them as a batch and returns multiple results
    pub async fn execute_query(
        &self,
        connection_id: &str,
        query: &str,
        database: Option<&str>,
        is_selection: bool,
        max_rows: Option<usize>,
    ) -> Result<Vec<QueryResult>, ConnectionError> {
        // Parse into statements
        let statements = parse_sql_statements(query);

        // If single statement, execute as before but return as Vec.
        // statement_index stays None (this is not a batch), but we still record
        // statement_text so the UI can label the result tab from the SQL that ran.
        if statements.len() == 1 {
            let results = self.execute_single_statement(
                connection_id,
                &statements[0],
                database,
                is_selection,
                max_rows,
                None,
                Some(statements[0].clone()),
            ).await?;
            return Ok(results);
        }

        // Multiple statements - execute as batch
        self.execute_batch(connection_id, statements, database, is_selection, max_rows).await
    }

    /// Execute a batch of SQL statements sequentially
    /// Note: If statements contains variable declarations (DECLARE/SET), they should
    /// already be combined into a single statement by parse_sql_statements
    async fn execute_batch(
        &self,
        connection_id: &str,
        statements: Vec<String>,
        database: Option<&str>,
        is_selection: bool,
        max_rows: Option<usize>,
    ) -> Result<Vec<QueryResult>, ConnectionError> {
        let mut results = Vec::new();

        for (index, statement) in statements.iter().enumerate() {
            // Check if batch was cancelled by looking at the results so far
            let query_result = self.execute_single_statement(
                connection_id,
                statement,
                database,
                is_selection,
                max_rows,
                Some(index),
                Some(statement.clone()),
            ).await;

            match query_result {
                Ok(mut statement_results) => {
                    // If batch cancelled, stop execution
                    let should_stop = statement_results
                        .iter()
                        .any(|r| r.error.as_ref().is_some_and(|e| e.contains("cancelled")));
                    results.append(&mut statement_results);
                    if should_stop {
                        break;
                    }
                }
                Err(e) => {
                    // Connection error - stop execution
                    results.push(QueryResult::with_error(
                        Uuid::new_v4().to_string(),
                        e.to_string()
                    ));
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Execute a single SQL statement and return result
    async fn execute_single_statement(
        &self,
        connection_id: &str,
        query: &str,
        database: Option<&str>,
        is_selection: bool,
        max_rows: Option<usize>,
        statement_index: Option<usize>,
        statement_text: Option<String>,
    ) -> Result<Vec<QueryResult>, ConnectionError> {
        let query_id = Uuid::new_v4().to_string();
        let start_time = std::time::Instant::now();
        
        log_info!("[QUERY] Starting query execution: query_id={}", query_id);
        
        // Create cancellation channel
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        
        // Store cancel sender for this query
        {
            let mut senders = self.cancel_senders.write().await;
            senders.insert(query_id.clone(), cancel_tx);
            log_info!("[QUERY] Stored cancel sender for query_id={}, total senders={}", query_id, senders.len());
            
            let mut info_map = self.query_info.write().await;
            info_map.insert(query_id.clone(), QueryInfo {
                query_id: query_id.clone(),
                connection_id: connection_id.to_string(),
                query: query.to_string(),
                status: QueryStatus::Running,
                started_at: Utc::now().to_rfc3339(),
                rows_fetched: 0,
            });
        }

        // Create a dedicated connection (not from pool) so we can drop it to cancel
        log_info!("[QUERY] Creating dedicated connection for query_id={}", query_id);
        let mut conn = self.connection_manager.create_dedicated_connection(connection_id).await?;
        log_info!("[QUERY] Dedicated connection created for query_id={}", query_id);

        // Detect final statement behavior (DML vs result set) for scripts such as
        // DECLARE ...; UPDATE ... that should report affected rows.
        let is_dml = infer_statement_kind(query) == StatementKind::Dml;

        let effective_max = max_rows
            .filter(|&m| m > 0)
            .unwrap_or(5000)
            .min(100_000);
        let row_limit = effective_max;
        let use_row_limit = !is_dml && row_limit > 0;
        let fetch_limit = row_limit.saturating_add(1);

        let statement_sql = if use_row_limit {
            format!("SET ROWCOUNT {}; {}; SET ROWCOUNT 0;", fetch_limit, query)
        } else {
            query.to_string()
        };

        // Build the full query with optional USE database — database identifier is validated + bracket-quoted
        let full_query = if let Some(db) = database {
            let use_db = crate::db::ident::resolve_database_name(db)
                .map_err(|e| ConnectionError::QueryError(e))?;
            format!("USE {use_db}; {statement_sql}")
        } else {
            statement_sql
        };

        log_info!("[QUERY] Executing query with tokio::select!, query_id={}", query_id);
        
        // Execute query with cancellation support using tokio::select!
        let query_id_for_log = query_id.clone();
        let query_future = async {
            log_info!("[QUERY] Query future started, query_id={}", query_id_for_log);
            if is_dml {
                let params: &[&dyn ToSql] = &[];
                let execute_result = conn
                    .execute(&full_query, params)
                    .await
                    .map_err(|e| ConnectionError::QueryError(e.to_string()))?;

                let affected_rows = execute_result
                    .rows_affected()
                    .iter()
                    .copied()
                    .sum::<u64>() as usize;

                log_info!(
                    "[QUERY] DML executed, affected_rows={}, query_id={}",
                    affected_rows,
                    query_id_for_log
                );

                Ok::<_, ConnectionError>((Vec::new(), affected_rows))
            } else {
                let stream = conn
                    .simple_query(&full_query)
                    .await
                    .map_err(|e| ConnectionError::QueryError(e.to_string()))?;

                log_info!("[QUERY] Query stream received, fetching results, query_id={}", query_id_for_log);
                // Use into_results() to get all result sets
                let all_results = stream
                    .into_results()
                    .await
                    .map_err(|e| ConnectionError::QueryError(e.to_string()))?;

                log_info!("[QUERY] Query results fetched, query_id={}", query_id_for_log);
                Ok::<_, ConnectionError>((all_results, 0usize))
            }
        };

        let query_id_for_cancel = query_id.clone();
        let result = tokio::select! {
            biased;
            
            // If cancel signal received, drop the connection and return cancelled
            _ = cancel_rx => {
                log_warn!("[QUERY] Cancel signal received! Dropping connection, query_id={}", query_id_for_cancel);
                // Drop the connection - this closes TCP and cancels the query on SQL Server
                drop(conn);
                
                let cancelled_result = self.make_cancelled_result(query_id, start_time, is_selection, statement_index, statement_text).await?;
                return Ok(vec![cancelled_result]);
            }
            
            // Normal query execution
            query_result = query_future => {
                log_info!("[QUERY] Query completed normally, query_id={}", query_id_for_cancel);
                query_result
            }
        };

        // Remove cancel sender (query completed)
        {
            let mut senders = self.cancel_senders.write().await;
            senders.remove(&query_id);
            log_info!("[QUERY] Removed cancel sender after completion, query_id={}", query_id);
        }

        match result {
            Ok((all_result_sets, affected_rows)) => {
                let execution_time = start_time.elapsed().as_millis() as u64;
 
                // For statements returning multiple result sets (e.g. DECLARE + multiple SELECTs),
                // return each non-empty result set as its own QueryResult.
                if !is_dml {
                    let mut multi_results: Vec<QueryResult> = Vec::new();
                    let mut total_rows_fetched = 0usize;

                    for (result_set_index, rows) in all_result_sets.iter().enumerate() {
                        if rows.is_empty() {
                            continue;
                        }

                        let columns: Vec<ColumnInfo> = rows[0].columns().iter().map(ColumnInfo::from).collect();
                        let col_types: Vec<ColumnType> = rows[0].columns().iter().map(|c| c.column_type()).collect();

                        let mut converted_rows: Vec<Vec<CellValue>> = rows
                            .iter()
                            .map(|row| {
                                (0..row.columns().len())
                                    .map(|idx| CellValue::from_row(row, idx, &col_types[idx]))
                                    .collect()
                            })
                            .collect();

                        let is_truncated = use_row_limit && converted_rows.len() > row_limit;
                        if is_truncated {
                            converted_rows.truncate(row_limit);
                        }
                        total_rows_fetched += converted_rows.len();

                        multi_results.push(QueryResult {
                            query_id: if result_set_index == 0 {
                                query_id.clone()
                            } else {
                                format!("{}:{}", query_id, result_set_index + 1)
                            },
                            columns,
                            row_count: converted_rows.len(),
                            rows: converted_rows,
                            truncated: is_truncated,
                            limit_applied: if use_row_limit { Some(row_limit) } else { None },
                            execution_time_ms: execution_time,
                            error: None,
                            is_complete: true,
                            is_selection,
                            statement_index,
                            statement_text: statement_text.clone(),
                        });
                    }

                    {
                        let mut info = self.query_info.write().await;
                        if let Some(qi) = info.get_mut(&query_id) {
                            qi.status = QueryStatus::Completed;
                            qi.rows_fetched = total_rows_fetched;
                        }
                    }

                    if !multi_results.is_empty() {
                        return Ok(multi_results);
                    }

                    return Ok(vec![QueryResult {
                        query_id,
                        columns: Vec::new(),
                        rows: Vec::new(),
                        row_count: 0,
                        truncated: false,
                        limit_applied: if use_row_limit { Some(row_limit) } else { None },
                        execution_time_ms: execution_time,
                        error: None,
                        is_complete: true,
                        is_selection,
                        statement_index,
                        statement_text,
                    }]);
                }

                {
                    let mut info = self.query_info.write().await;
                    if let Some(qi) = info.get_mut(&query_id) {
                        qi.status = QueryStatus::Completed;
                        qi.rows_fetched = affected_rows;
                    }
                }

                Ok(vec![QueryResult {
                    query_id,
                    columns: Vec::new(),
                    rows: Vec::new(),
                    row_count: affected_rows,
                    truncated: false,
                    limit_applied: None,
                    execution_time_ms: execution_time,
                    error: None,
                    is_complete: true,
                    is_selection,
                    statement_index,
                    statement_text,
                }])
            }
            Err(e) => {
                // Check if error is due to cancellation
                let error_msg = e.to_string();
                if error_msg.contains("connection closed") || error_msg.contains("reset") {
                    let cancelled_result = self.make_cancelled_result(query_id, start_time, is_selection, statement_index, statement_text).await?;
                    return Ok(vec![cancelled_result]);
                }
                
                // Update query info
                {
                    let mut info = self.query_info.write().await;
                    if let Some(qi) = info.get_mut(&query_id) {
                        qi.status = QueryStatus::Error;
                    }
                }

                Ok(vec![QueryResult::with_error(query_id, error_msg)])
            }
        }
    }
    
    /// Helper to create a cancelled result
    async fn make_cancelled_result(
        &self,
        query_id: String,
        start_time: std::time::Instant,
        is_selection: bool,
        statement_index: Option<usize>,
        statement_text: Option<String>,
    ) -> Result<QueryResult, ConnectionError> {
        // Update query info
        {
            let mut info = self.query_info.write().await;
            if let Some(qi) = info.get_mut(&query_id) {
                qi.status = QueryStatus::Cancelled;
            }
        }
        
        // Remove cancel sender
        {
            let mut senders = self.cancel_senders.write().await;
            senders.remove(&query_id);
        }
        
        Ok(QueryResult {
            query_id,
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
            truncated: false,
            limit_applied: None,
            execution_time_ms: start_time.elapsed().as_millis() as u64,
            error: Some("Query cancelled".to_string()),
            is_complete: true,
            is_selection,
            statement_index,
            statement_text,
        })
    }

    /// Cancel a running query by sending cancel signal
    pub async fn cancel_query(&self, query_id: &str) -> bool {
        log_info!("[CANCEL] cancel_query called with query_id={}", query_id);
        
        let mut senders = self.cancel_senders.write().await;
        let all_keys: Vec<String> = senders.keys().cloned().collect();
        log_info!("[CANCEL] Current cancel senders: {:?}", all_keys);
        
        if let Some(cancel_tx) = senders.remove(query_id) {
            log_info!("[CANCEL] Found cancel sender for query_id={}, sending cancel signal", query_id);
            // Send cancel signal - this will trigger the select! branch
            match cancel_tx.send(()) {
                Ok(_) => log_info!("[CANCEL] Cancel signal sent successfully for query_id={}", query_id),
                Err(_) => log_warn!("[CANCEL] Failed to send cancel signal (receiver dropped) for query_id={}", query_id),
            }
            
            // Update query status to cancelled
            let mut info = self.query_info.write().await;
            if let Some(qi) = info.get_mut(query_id) {
                qi.status = QueryStatus::Cancelled;
            }
            
            true
        } else {
            log_warn!("[CANCEL] No cancel sender found for query_id={}", query_id);
            false
        }
    }

    /// Cancel all running queries for a connection (useful when we don't have the query_id)
    pub async fn cancel_all_for_connection(&self, connection_id: &str) -> usize {
        log_info!("[CANCEL] cancel_all_for_connection called with connection_id={}", connection_id);
        
        // Find all query_ids for this connection
        let query_ids: Vec<String> = {
            let info = self.query_info.read().await;
            info.iter()
                .filter(|(_, qi)| qi.connection_id == connection_id && qi.status == QueryStatus::Running)
                .map(|(id, _)| id.clone())
                .collect()
        };
        
        log_info!("[CANCEL] Found {} running queries for connection_id={}: {:?}", query_ids.len(), connection_id, query_ids);
        
        let mut cancelled_count = 0;
        for query_id in query_ids {
            if self.cancel_query(&query_id).await {
                cancelled_count += 1;
            }
        }
        
        log_info!("[CANCEL] Cancelled {} queries for connection_id={}", cancelled_count, connection_id);
        cancelled_count
    }

    /// Get status of a query
    pub async fn get_query_status(&self, query_id: &str) -> Option<QueryInfo> {
        let info = self.query_info.read().await;
        info.get(query_id).cloned()
    }

    /// Clean up old query info (call periodically)
    pub async fn cleanup_old_queries(&self) {
        let mut info = self.query_info.write().await;
        info.retain(|_, qi| {
            qi.status == QueryStatus::Running || qi.status == QueryStatus::Pending
        });
    }
}
