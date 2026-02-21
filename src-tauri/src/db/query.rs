// Query Execution Engine (T017)
// Handles non-blocking query execution with result streaming

use crate::db::connection::{ConnectionError, MssqlConnectionManager};
use crate::db::managers::UnifiedConnectionManager;
use crate::db::traits::{DatabaseType, DatabaseDriver, DatabaseConfig, CellValue, QueryResult, ColumnInfo};
use crate::db::drivers::mssql::MssqlDriver;
use crate::db::drivers::postgres::PostgresDriver;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tiberius::{Column, ColumnType, Row};
use tiberius::numeric::Numeric;
use tokio::sync::{RwLock, oneshot, Mutex};
use uuid::Uuid;

// Logging macros using println for simplicity (no trailing semicolon for use in match arms)
macro_rules! log_info {
    ($($arg:tt)*) => {{ println!("[INFO] {}", format!($($arg)*)) }};
}
macro_rules! log_warn {
    ($($arg:tt)*) => {{ println!("[WARN] {}", format!($($arg)*)) }};
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

/// Tracks active queries for cancellation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryInfo {
    pub id: String,
    pub connection_id: String,
    pub sql: String,
    pub start_time: DateTime<Utc>,
    pub is_cancellable: bool,
}

/// Query execution engine
pub struct QueryEngine {
    connection_manager: Arc<UnifiedConnectionManager>,
    /// Track active queries for cancellation
    active_queries: Mutex<HashMap<String, QueryInfo>>,
    // Drivers for execution logic
    mssql_driver: MssqlDriver,
    postgres_driver: PostgresDriver,
}

impl QueryEngine {
    pub fn new(connection_manager: Arc<UnifiedConnectionManager>) -> Self {
        Self {
            connection_manager,
            active_queries: Mutex::new(HashMap::new()),
            mssql_driver: MssqlDriver::new(),
            postgres_driver: PostgresDriver::new(),
        }
    }

    /// Split a script into individual batches/statements
    /// Basic implementation: split by GO or ; depending on dialect
    fn split_script(&self, script: &str, db_type: DatabaseType) -> Vec<String> {
        match db_type {
            DatabaseType::Mssql => {
                // MS-SQL splits by GO
                script.split("\nGO\n")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            },
            DatabaseType::Postgresql => {
                // Postgres splits by semi-colon (simplified)
                vec![script.trim().to_string()]
            },
            _ => vec![script.trim().to_string()],
        }
    }

    /// Execute a query script (potentially multiple batches)
    pub async fn execute_query(
        &self,
        connection_id: &str,
        query: &str,
        database: Option<&str>, // Optional context switch
        _is_selection: bool, // If true, this is a "run selection" action
    ) -> Result<Vec<QueryResult>, ConnectionError> {
        let db_type = self.connection_manager.get_connection_type(connection_id).await
            .unwrap_or(DatabaseType::Mssql);
            
        // 1. Split script
        let batches = self.split_script(query, db_type.clone());
        let mut results = Vec::new();

        // 2. Execute each batch
        for (_i, sql) in batches.iter().enumerate() {
             let query_id = Uuid::new_v4().to_string();
             
             // Track query
             {
                 let mut active = self.active_queries.lock().await;
                 active.insert(query_id.clone(), QueryInfo {
                     id: query_id.clone(),
                     connection_id: connection_id.to_string(),
                     sql: sql.clone(),
                     start_time: Utc::now(),
                     is_cancellable: true,
                 });
             }

             // Execute based on driver
             let result = match db_type {
                 DatabaseType::Mssql => {
                     // Get connection from pool and wrap it
                     let pool = self.connection_manager.mssql().connect(connection_id).await
                        .map_err(|e| ConnectionError::PoolError(e.to_string()))?;
                     
                     let conn_wrapper = crate::db::drivers::mssql::MssqlConnection {
                         id: connection_id.to_string(),
                         pool,
                     };
                     
                     self.mssql_driver.execute_query(&conn_wrapper, sql, query_id.clone(), database).await
                 },
                 DatabaseType::Postgresql => {
                     let pool = self.connection_manager.postgres().connect(connection_id).await
                        .map_err(|e| ConnectionError::PoolError(e.to_string()))?;
                        
                     let conn_wrapper = crate::db::drivers::postgres::PostgresConnection {
                         id: connection_id.to_string(),
                         pool,
                     };
                     
                     self.postgres_driver.execute_query(&conn_wrapper, sql, query_id.clone(), database).await
                 },
                 DatabaseType::Sqlite => {
                     // SQLite: open connection per-query from stored config (stateless)
                     use crate::db::traits::DatabaseDriver;
                     let driver_result: Result<crate::db::traits::QueryResult, crate::db::DatabaseError> = async {
                         let db_config = self.connection_manager.sqlite()
                             .get_database_config(connection_id).await
                             .ok_or_else(|| crate::db::DatabaseError::QueryError(
                                 format!("SQLite connection config not found: {}", connection_id)
                             ))?;
                         let sqlite_driver = crate::db::drivers::sqlite::SqliteDriver::new();
                         let conn = sqlite_driver.connect(&db_config).await?;
                         sqlite_driver.execute_query(conn.as_ref(), sql, query_id.clone(), database).await
                     }.await;
                     driver_result
                 },
                 _ => Err(crate::db::DatabaseError::QueryError("Unsupported database type".to_string())),
             };
             
             // Untrack query
             {
                 let mut active = self.active_queries.lock().await;
                 active.remove(&query_id);
             }
             
             match result {
                 Ok(res) => results.push(res),
                 Err(e) => return Err(ConnectionError::QueryError(e.to_string())),
             }
        }

        Ok(results)
    }

    /// Cancel a running query
    pub async fn cancel_query(&self, query_id: &str) -> Option<bool> {
        // In the new driver-based architecture, cancellation depends on the driver offering a handle.
        // For now, we just remove it from tracking to indicate "cancelled" in UI if possible,
        // but actual backend cancellation requires driver support (e.g. collecting PIDs).
        // Phase 2 will add proper cancellation.
        let mut active = self.active_queries.lock().await;
        if active.remove(query_id).is_some() {
             // We found it and removed it.
             // Ideally we'd signal the driver.
             Some(true)
        } else {
             Some(false)
        }
    }
    /// Cancel all running queries for a connection
    pub async fn cancel_all_for_connection(&self, connection_id: &str) -> usize {
        let mut active = self.active_queries.lock().await;
        // Collect IDs first to avoid borrowing issues if we were calling a method, 
        // but here we just remove.
        let ids: Vec<String> = active.iter()
            .filter(|(_, info)| info.connection_id == connection_id)
            .map(|(id, _)| id.clone())
            .collect();
            
        let count = ids.len();
        for id in ids {
            active.remove(&id);
        }
        
        count
    }

    /// Get status of a query
    pub async fn get_query_status(&self, query_id: &str) -> Option<QueryInfo> {
        let active = self.active_queries.lock().await;
        // active_queries holds running queries. If not found, it's unknown or finished.
        // The previous implementation tracked history?
        // For now, simple return.
        // We need to Clone QueryInfo.
        // QueryInfo derives Clone.
        active.get(query_id).cloned()
    }
}
