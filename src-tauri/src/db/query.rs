// Query Execution Engine (T017)
// Handles non-blocking query execution with result streaming

use crate::db::connection::{ConnectionError, MssqlConnectionManager};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tiberius::{Column, ColumnType, Row};
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

/// Parse SQL text into individual statements
/// Splits on:
/// - GO keyword (SQL Server batch separator, case-insensitive, on its own line)
/// - Semicolons (standard SQL statement terminator)
/// 
/// Special handling: If the query starts with DECLARE or SET (variable/parameter declarations),
/// the entire declaration block plus all following statements are kept together as one unit.
/// This preserves variable scope across multiple statements.
/// 
/// Auto-wrapping: If a statement is just a procedure name (e.g., "sp_who2"), it's automatically
/// wrapped with EXEC to allow execution without explicit EXEC keyword (like SSMS/DBeaver).
fn parse_sql_statements(sql: &str) -> Vec<String> {
    let trimmed_sql = sql.trim();
    let sql_upper = trimmed_sql.to_uppercase();
    
    // Check if query starts with DECLARE or SET (variable/parameter declarations)
    let has_declarations = sql_upper.starts_with("DECLARE") || sql_upper.starts_with("SET");
    
    // If there are declarations, keep the entire batch as one statement
    // This ensures variable scope is preserved across all statements
    if has_declarations {
        return vec![auto_wrap_procedure(trimmed_sql.to_string())];
    }
    
    let mut statements = Vec::new();
    let mut current_statement = String::new();
    let mut in_string = false;
    let mut in_comment = false;
    let mut in_line_comment = false;
    let mut prev_char = '\0';

    for line in sql.lines() {
        let trimmed = line.trim();

        // Check for GO keyword (only if not in string/comment)
        if !in_string && !in_comment && !in_line_comment {
            if trimmed.eq_ignore_ascii_case("go") {
                // End current statement
                let stmt = current_statement.trim().to_string();
                if !stmt.is_empty() {
                    statements.push(stmt);
                }
                current_statement.clear();
                continue;
            }
        }

        // Process line character by character for semicolon detection
        for ch in line.chars() {
            // Toggle string state
            if ch == '\'' && prev_char != '\\' && !in_comment && !in_line_comment {
                in_string = !in_string;
            }

            // Check for comment start
            if !in_string {
                if ch == '-' && prev_char == '-' && !in_comment {
                    in_line_comment = true;
                } else if ch == '*' && prev_char == '/' && !in_line_comment {
                    in_comment = true;
                } else if ch == '/' && prev_char == '*' && in_comment {
                    in_comment = false;
                }
            }

            // Check for semicolon (statement terminator)
            if ch == ';' && !in_string && !in_comment && !in_line_comment {
                current_statement.push(ch);
                let stmt = current_statement.trim().to_string();
                if !stmt.is_empty() && stmt != ";" {
                    statements.push(auto_wrap_procedure(stmt));
                }
                current_statement.clear();
                prev_char = ch;
                continue;
            }

            current_statement.push(ch);
            prev_char = ch;
        }

        // Reset line comment at end of line
        in_line_comment = false;
        current_statement.push('\n');
    }

    // Add final statement if any
    let final_stmt = current_statement.trim().to_string();
    if !final_stmt.is_empty() {
        statements.push(auto_wrap_procedure(final_stmt));
    }

    // If no statements were parsed, return the original as single statement
    if statements.is_empty() && !sql.trim().is_empty() {
        statements.push(auto_wrap_procedure(sql.trim().to_string()));
    }

    statements
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
    ) -> Result<Vec<QueryResult>, ConnectionError> {
        // Parse into statements
        let statements = parse_sql_statements(query);

        // If single statement, execute as before but return as Vec
        if statements.len() == 1 {
            let result = self.execute_single_statement(
                connection_id,
                &statements[0],
                database,
                is_selection,
                None,
                None,
            ).await?;
            return Ok(vec![result]);
        }

        // Multiple statements - execute as batch
        self.execute_batch(connection_id, statements, database, is_selection).await
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
    ) -> Result<Vec<QueryResult>, ConnectionError> {
        let mut results = Vec::new();

        for (index, statement) in statements.iter().enumerate() {
            // Check if batch was cancelled by looking at the results so far
            let query_result = self.execute_single_statement(
                connection_id,
                statement,
                database,
                is_selection,
                Some(index),
                Some(statement.clone()),
            ).await;

            match query_result {
                Ok(result) => {
                    // If batch cancelled, stop execution
                    let should_stop = result.error.as_ref().map_or(false, |e| e.contains("cancelled"));
                    results.push(result);
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
        statement_index: Option<usize>,
        statement_text: Option<String>,
    ) -> Result<QueryResult, ConnectionError> {
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

        // Check if this is a DML query (INSERT, UPDATE, DELETE, MERGE)
        let query_upper = query.trim().to_uppercase();
        let is_dml = query_upper.starts_with("UPDATE") 
            || query_upper.starts_with("INSERT") 
            || query_upper.starts_with("DELETE")
            || query_upper.starts_with("MERGE");

        // Build the full query with optional USE database
        let full_query = if let Some(db) = database {
            format!("USE [{}]; {}", db, query)
        } else {
            query.to_string()
        };

        log_info!("[QUERY] Executing query with tokio::select!, query_id={}", query_id);
        
        // Execute query with cancellation support using tokio::select!
        let query_id_for_log = query_id.clone();
        let query_future = async {
            log_info!("[QUERY] Query future started, query_id={}", query_id_for_log);
            let stream = conn.simple_query(&full_query).await
                .map_err(|e| ConnectionError::QueryError(e.to_string()))?;
            
            log_info!("[QUERY] Query stream received, fetching results, query_id={}", query_id_for_log);
            // Use into_results() to get all result sets
            let all_results = stream.into_results().await
                .map_err(|e| ConnectionError::QueryError(e.to_string()))?;
            
            log_info!("[QUERY] Query results fetched, query_id={}", query_id_for_log);
            Ok::<_, ConnectionError>(all_results)
        };

        let query_id_for_cancel = query_id.clone();
        let result = tokio::select! {
            biased;
            
            // If cancel signal received, drop the connection and return cancelled
            _ = cancel_rx => {
                log_warn!("[QUERY] Cancel signal received! Dropping connection, query_id={}", query_id_for_cancel);
                // Drop the connection - this closes TCP and cancels the query on SQL Server
                drop(conn);
                
                return self.make_cancelled_result(query_id, start_time, is_selection, statement_index, statement_text).await;
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
            Ok(all_result_sets) => {
                let execution_time = start_time.elapsed().as_millis() as u64;
                
                // For DML queries, show a simple success message
                // For SELECT queries, show actual data rows
                let (columns, converted_rows) = if is_dml {
                    let column = ColumnInfo {
                        name: "Result".to_string(),
                        data_type: "String".to_string(),
                        nullable: false,
                    };
                    let row = vec![CellValue::String("Query executed successfully".to_string())];
                    (vec![column], vec![row])
                } else if !all_result_sets.is_empty() && !all_result_sets[0].is_empty() {
                    let rows = &all_result_sets[0];
                    
                    // Extract column info from first row
                    let columns: Vec<ColumnInfo> = rows[0].columns().iter().map(ColumnInfo::from).collect();
                    let col_types: Vec<ColumnType> = rows[0].columns().iter().map(|c| c.column_type()).collect();

                    // Convert rows to our format
                    let converted_rows: Vec<Vec<CellValue>> = rows
                        .iter()
                        .map(|row| {
                            (0..row.columns().len())
                                .map(|idx| CellValue::from_row(row, idx, &col_types[idx]))
                                .collect()
                        })
                        .collect();
                    
                    (columns, converted_rows)
                } else {
                    (Vec::new(), Vec::new())
                };

                let row_count = converted_rows.len();

                // Update query info
                {
                    let mut info = self.query_info.write().await;
                    if let Some(qi) = info.get_mut(&query_id) {
                        qi.status = QueryStatus::Completed;
                        qi.rows_fetched = row_count;
                    }
                }

                Ok(QueryResult {
                    query_id,
                    columns,
                    rows: converted_rows,
                    row_count,
                    execution_time_ms: execution_time,
                    error: None,
                    is_complete: true,
                    is_selection,
                    statement_index,
                    statement_text,
                })
            }
            Err(e) => {
                // Check if error is due to cancellation
                let error_msg = e.to_string();
                if error_msg.contains("connection closed") || error_msg.contains("reset") {
                    return self.make_cancelled_result(query_id, start_time, is_selection, statement_index, statement_text).await;
                }
                
                // Update query info
                {
                    let mut info = self.query_info.write().await;
                    if let Some(qi) = info.get_mut(&query_id) {
                        qi.status = QueryStatus::Error;
                    }
                }

                Ok(QueryResult::with_error(query_id, error_msg))
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
