// Query Execution Engine (T017)
// Driver-neutral: splits input via the connection's driver, runs each statement
// with cancellation support, and shapes driver output into `QueryResult`s.

use crate::db::driver::{ExecuteOptions, ExecutionOutput};
use crate::db::error::ConnectionError;
use crate::db::manager::ConnectionManager;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};
use uuid::Uuid;

/// Rows returned per result set when the caller does not ask for a limit.
const DEFAULT_ROW_LIMIT: usize = 5000;
/// Hard ceiling on rows returned per result set.
const MAX_ROW_LIMIT: usize = 100_000;

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

/// Column metadata for the result set
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
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

/// Normalize a caller-supplied row limit: 0/None means the default, capped at the max.
fn effective_row_limit(max_rows: Option<usize>) -> usize {
    max_rows
        .filter(|&m| m > 0)
        .unwrap_or(DEFAULT_ROW_LIMIT)
        .min(MAX_ROW_LIMIT)
}

/// Turn a driver's output into the `QueryResult`s shown as result tabs.
/// Returns the results plus the total row count for status bookkeeping.
fn shape_results(
    output: ExecutionOutput,
    query_id: &str,
    execution_time_ms: u64,
    is_selection: bool,
    statement_index: Option<usize>,
    statement_text: Option<String>,
) -> (Vec<QueryResult>, usize) {
    match output {
        ExecutionOutput::RowsAffected(affected_rows) => (
            vec![QueryResult {
                query_id: query_id.to_string(),
                columns: Vec::new(),
                rows: Vec::new(),
                row_count: affected_rows,
                truncated: false,
                limit_applied: None,
                execution_time_ms,
                error: None,
                is_complete: true,
                is_selection,
                statement_index,
                statement_text,
            }],
            affected_rows,
        ),
        ExecutionOutput::ResultSets { sets, row_limit } => {
            // Statements returning multiple result sets (e.g. DECLARE + multiple
            // SELECTs) get one QueryResult per non-empty result set.
            let mut results: Vec<QueryResult> = Vec::new();
            let mut total_rows = 0usize;

            for (result_set_index, set) in sets.into_iter().enumerate() {
                if set.rows.is_empty() {
                    continue;
                }
                total_rows += set.rows.len();
                results.push(QueryResult {
                    query_id: if result_set_index == 0 {
                        query_id.to_string()
                    } else {
                        format!("{}:{}", query_id, result_set_index + 1)
                    },
                    columns: set.columns,
                    row_count: set.rows.len(),
                    rows: set.rows,
                    truncated: set.truncated,
                    limit_applied: row_limit,
                    execution_time_ms,
                    error: None,
                    is_complete: true,
                    is_selection,
                    statement_index,
                    statement_text: statement_text.clone(),
                });
            }

            if results.is_empty() {
                results.push(QueryResult {
                    query_id: query_id.to_string(),
                    columns: Vec::new(),
                    rows: Vec::new(),
                    row_count: 0,
                    truncated: false,
                    limit_applied: row_limit,
                    execution_time_ms,
                    error: None,
                    is_complete: true,
                    is_selection,
                    statement_index,
                    statement_text,
                });
            }

            (results, total_rows)
        }
    }
}

/// Query execution engine
pub struct QueryEngine {
    connection_manager: Arc<ConnectionManager>,
    /// Track running query cancellation senders - sending cancels the query
    cancel_senders: RwLock<HashMap<String, oneshot::Sender<()>>>,
    /// Query info for status checking
    query_info: RwLock<HashMap<String, QueryInfo>>,
}

impl QueryEngine {
    pub fn new(connection_manager: Arc<ConnectionManager>) -> Self {
        Self {
            connection_manager,
            cancel_senders: RwLock::new(HashMap::new()),
            query_info: RwLock::new(HashMap::new()),
        }
    }

    /// Execute a query (single or batch) and return results
    /// If the driver splits the input into multiple statements, executes them
    /// as a batch and returns multiple results
    pub async fn execute_query(
        &self,
        connection_id: &str,
        query: &str,
        database: Option<&str>,
        is_selection: bool,
        max_rows: Option<usize>,
    ) -> Result<Vec<QueryResult>, ConnectionError> {
        let (driver, _) = self.connection_manager.resolve(connection_id).await?;
        let statements = driver.split_statements(query);

        // If single statement, execute as before but return as Vec.
        // statement_index stays None (this is not a batch), but we still record
        // statement_text so the UI can label the result tab from the SQL that ran.
        if statements.len() == 1 {
            let results = self
                .execute_single_statement(
                    connection_id,
                    &statements[0],
                    database,
                    is_selection,
                    max_rows,
                    None,
                    Some(statements[0].clone()),
                )
                .await?;
            return Ok(results);
        }

        // Multiple statements - execute as batch
        self.execute_batch(connection_id, statements, database, is_selection, max_rows)
            .await
    }

    /// Execute a batch of statements sequentially
    /// Note: statements that must share scope (e.g. T-SQL variables) are expected
    /// to already be combined into a single statement by the driver's splitter
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
            let query_result = self
                .execute_single_statement(
                    connection_id,
                    statement,
                    database,
                    is_selection,
                    max_rows,
                    Some(index),
                    Some(statement.clone()),
                )
                .await;

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
                        e.to_string(),
                    ));
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Execute a single statement and return result
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

        let (driver, config) = self.connection_manager.resolve(connection_id).await?;

        // Create cancellation channel
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        // Store cancel sender for this query
        {
            let mut senders = self.cancel_senders.write().await;
            senders.insert(query_id.clone(), cancel_tx);
            log_info!(
                "[QUERY] Stored cancel sender for query_id={}, total senders={}",
                query_id,
                senders.len()
            );

            let mut info_map = self.query_info.write().await;
            info_map.insert(
                query_id.clone(),
                QueryInfo {
                    query_id: query_id.clone(),
                    connection_id: connection_id.to_string(),
                    query: query.to_string(),
                    status: QueryStatus::Running,
                    started_at: Utc::now().to_rfc3339(),
                    rows_fetched: 0,
                },
            );
        }

        let options = ExecuteOptions {
            database,
            row_limit: effective_row_limit(max_rows),
        };

        log_info!(
            "[QUERY] Executing query with tokio::select!, query_id={}",
            query_id
        );

        // Execute with cancellation support. The driver owns its connection inside
        // the future, so dropping the future on cancel tears the connection down.
        let result = tokio::select! {
            biased;

            _ = cancel_rx => {
                log_warn!("[QUERY] Cancel signal received! Dropping query future, query_id={}", query_id);
                let cancelled_result = self.make_cancelled_result(query_id, start_time, is_selection, statement_index, statement_text).await?;
                return Ok(vec![cancelled_result]);
            }

            query_result = driver.execute(&config, query, options) => {
                log_info!("[QUERY] Query completed normally, query_id={}", query_id);
                query_result
            }
        };

        // Remove cancel sender (query completed)
        {
            let mut senders = self.cancel_senders.write().await;
            senders.remove(&query_id);
            log_info!(
                "[QUERY] Removed cancel sender after completion, query_id={}",
                query_id
            );
        }

        match result {
            Ok(output) => {
                let execution_time = start_time.elapsed().as_millis() as u64;
                let (results, rows_fetched) = shape_results(
                    output,
                    &query_id,
                    execution_time,
                    is_selection,
                    statement_index,
                    statement_text,
                );

                let mut info = self.query_info.write().await;
                if let Some(qi) = info.get_mut(&query_id) {
                    qi.status = QueryStatus::Completed;
                    qi.rows_fetched = rows_fetched;
                }

                Ok(results)
            }
            Err(ConnectionError::Cancelled) => {
                let cancelled_result = self
                    .make_cancelled_result(
                        query_id,
                        start_time,
                        is_selection,
                        statement_index,
                        statement_text,
                    )
                    .await?;
                Ok(vec![cancelled_result])
            }
            Err(e) => {
                // Update query info
                {
                    let mut info = self.query_info.write().await;
                    if let Some(qi) = info.get_mut(&query_id) {
                        qi.status = QueryStatus::Error;
                    }
                }

                match e {
                    // The statement itself failed — report it in the result tab
                    ConnectionError::QueryError(_) => {
                        Ok(vec![QueryResult::with_error(query_id, e.to_string())])
                    }
                    // Connection-level failure (unreachable, auth, password expired)
                    _ => Err(e),
                }
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
            log_info!(
                "[CANCEL] Found cancel sender for query_id={}, sending cancel signal",
                query_id
            );
            // Send cancel signal - this will trigger the select! branch
            match cancel_tx.send(()) {
                Ok(_) => log_info!(
                    "[CANCEL] Cancel signal sent successfully for query_id={}",
                    query_id
                ),
                Err(_) => log_warn!(
                    "[CANCEL] Failed to send cancel signal (receiver dropped) for query_id={}",
                    query_id
                ),
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
        log_info!(
            "[CANCEL] cancel_all_for_connection called with connection_id={}",
            connection_id
        );

        // Find all query_ids for this connection
        let query_ids: Vec<String> = {
            let info = self.query_info.read().await;
            info.iter()
                .filter(|(_, qi)| {
                    qi.connection_id == connection_id && qi.status == QueryStatus::Running
                })
                .map(|(id, _)| id.clone())
                .collect()
        };

        log_info!(
            "[CANCEL] Found {} running queries for connection_id={}: {:?}",
            query_ids.len(),
            connection_id,
            query_ids
        );

        let mut cancelled_count = 0;
        for query_id in query_ids {
            if self.cancel_query(&query_id).await {
                cancelled_count += 1;
            }
        }

        log_info!(
            "[CANCEL] Cancelled {} queries for connection_id={}",
            cancelled_count,
            connection_id
        );
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
        info.retain(|_, qi| qi.status == QueryStatus::Running || qi.status == QueryStatus::Pending);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::driver::ResultSet;

    fn set(rows: usize, truncated: bool) -> ResultSet {
        ResultSet {
            columns: vec![ColumnInfo {
                name: "x".to_string(),
                data_type: "int".to_string(),
                nullable: true,
            }],
            rows: (0..rows).map(|i| vec![CellValue::Int(i as i64)]).collect(),
            truncated,
        }
    }

    #[test]
    fn row_limit_defaults_and_caps() {
        assert_eq!(effective_row_limit(None), DEFAULT_ROW_LIMIT);
        assert_eq!(effective_row_limit(Some(0)), DEFAULT_ROW_LIMIT);
        assert_eq!(effective_row_limit(Some(10)), 10);
        assert_eq!(effective_row_limit(Some(1_000_000)), MAX_ROW_LIMIT);
    }

    #[test]
    fn rows_affected_becomes_single_result() {
        let (results, rows) = shape_results(
            ExecutionOutput::RowsAffected(7),
            "q",
            5,
            false,
            Some(2),
            Some("UPDATE t SET x = 1".to_string()),
        );
        assert_eq!(rows, 7);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].row_count, 7);
        assert_eq!(results[0].limit_applied, None);
        assert_eq!(results[0].statement_index, Some(2));
    }

    #[test]
    fn empty_result_sets_are_skipped_and_ids_suffixed() {
        let (results, rows) = shape_results(
            ExecutionOutput::ResultSets {
                sets: vec![set(2, false), set(0, false), set(3, true)],
                row_limit: Some(3),
            },
            "q",
            5,
            true,
            None,
            None,
        );
        assert_eq!(rows, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].query_id, "q");
        assert_eq!(results[1].query_id, "q:3");
        assert!(results[1].truncated);
        assert_eq!(results[1].limit_applied, Some(3));
        assert!(results.iter().all(|r| r.is_selection));
    }

    #[test]
    fn no_rows_yields_one_empty_result() {
        let (results, rows) = shape_results(
            ExecutionOutput::ResultSets {
                sets: vec![set(0, false)],
                row_limit: Some(5000),
            },
            "q",
            5,
            false,
            None,
            None,
        );
        assert_eq!(rows, 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].row_count, 0);
        assert_eq!(results[0].limit_applied, Some(5000));
    }
}
