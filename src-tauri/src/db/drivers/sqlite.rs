// SQLite Driver (Phase 2.5 - Before Phase 3)
// Implements DatabaseDriver trait for SQLite using rusqlite

use crate::db::traits::{
    DatabaseDriver, DatabaseType, DatabaseError, DatabaseConfig, Connection,
    QueryResult, TableInfo, ColumnInfo, CellValue,
};
use rusqlite::{Connection as RusqliteConnection, params, OpenFlags};
use std::sync::Arc;
use std::path::Path;

/// SQLite specific connection wrapper
pub struct SqliteConnection {
    pub id: String,
    pub conn: Arc<tokio::sync::Mutex<RusqliteConnection>>,
}

#[async_trait::async_trait]
impl Connection for SqliteConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }

    async fn is_alive(&self) -> bool {
        match self.conn.lock().await.execute("SELECT 1", []) {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// SQLite driver implementation
pub struct SqliteDriver;

impl SqliteDriver {
    /// Create a new SQLite driver
    pub fn new() -> Self {
        Self
    }

    /// Extract database path from config
    fn get_database_path(config: &DatabaseConfig) -> Result<String, DatabaseError> {
        if config.database.is_empty() {
            return Err(DatabaseError::InvalidConfig(
                "SQLite database path is required".to_string(),
            ));
        }

        // Expand ~ to home directory if present
        let path = if config.database.starts_with("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                Path::new(&home)
                    .join(&config.database[2..])
                    .to_string_lossy()
                    .to_string()
            } else {
                config.database.clone()
            }
        } else {
            config.database.clone()
        };

        Ok(path)
    }

    /// Open SQLite connection
    fn open_connection(path: &str) -> Result<RusqliteConnection, DatabaseError> {
        RusqliteConnection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE)
            .map_err(|e| DatabaseError::ConnectionFailed(format!("Failed to open SQLite database: {}", e)))
    }
}

impl Default for SqliteDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl DatabaseDriver for SqliteDriver {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::Sqlite
    }

    async fn test_connection(&self, config: &DatabaseConfig) -> Result<bool, DatabaseError> {
        let path = Self::get_database_path(config)?;
        let test_conn = Self::open_connection(&path);
        Ok(test_conn.is_ok())
    }

    async fn connect(&self, config: &DatabaseConfig) -> Result<Box<dyn Connection>, DatabaseError> {
        let path = Self::get_database_path(config)?;
        let sqlite_conn = Self::open_connection(&path)?;

        let connection = SqliteConnection {
            id: config.id.clone(),
            conn: Arc::new(tokio::sync::Mutex::new(sqlite_conn)),
        };

        Ok(Box::new(connection))
    }

    async fn execute_query(
        &self,
        conn: &dyn Connection,
        sql: &str,
        query_id: String,
        _database: Option<&str>,
    ) -> Result<QueryResult, DatabaseError> {
        let sqlite_conn = conn
            .as_any()
            .downcast_ref::<SqliteConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;

        let conn_guard = sqlite_conn.conn.lock().await;
        let conn_ref = &*conn_guard;
        let start = std::time::Instant::now();

        let mut result = QueryResult::new(query_id.clone());
        result.statement_text = Some(sql.to_string());

        // Check if query is a SELECT query
        let is_select = sql.trim().to_uppercase().starts_with("SELECT");

        if is_select {
            // Execute SELECT query
            let mut stmt = conn_ref.prepare(sql)
                .map_err(|e| DatabaseError::QueryError(format!("Failed to prepare query: {}", e)))?;

            // Get column info from statement
            let column_count = stmt.column_count();
            let column_names = stmt.column_names();
            result.columns = (0..column_count)
                .map(|idx| ColumnInfo {
                    name: column_names.get(idx).map(|s| s.to_string()).unwrap_or(format!("col_{}", idx)),
                    data_type: "any".to_string(),  // SQLite is dynamically typed
                    max_length: None,
                    precision: None,
                    scale: None,
                    is_nullable: true,  // SQLite is dynamically typed
                    is_primary_key: false,
                    is_identity: false,
                    column_default: None,
                    ordinal_position: idx as i32,
                })
                .collect();

            // Execute and fetch rows
            let mut row_vec = Vec::new();
            let mut rows = stmt.query([]).map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            while let Some(row) = rows.next().map_err(|e| DatabaseError::QueryError(e.to_string()))? {
                let cell_values: Vec<CellValue> = result
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(idx, _)| Self::cell_value_from_row(&row, idx as usize))
                    .collect();
                row_vec.push(cell_values);
            }
            result.rows = row_vec;

            result.row_count = result.rows.len();
        } else {
            // Execute DML/DDL query
            conn_ref.execute(sql, [])
                .map_err(|e| DatabaseError::QueryError(format!("Query execution failed: {}", e)))?;

            // For non-SELECT queries, return a success message
            result.columns = vec![ColumnInfo {
                name: "Result".to_string(),
                data_type: "String".to_string(),
                max_length: None,
                precision: None,
                scale: None,
                is_nullable: false,
                is_primary_key: false,
                is_identity: false,
                column_default: None,
                ordinal_position: 0,
            }];
            result.rows = vec![vec![CellValue::String("Query executed successfully".to_string())]];
            result.row_count = 1;
        }

        result.is_complete = true;
        result.execution_time_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }

    async fn cancel_query(&self, _query_id: &str) -> Result<(), DatabaseError> {
        // SQLite doesn't support query cancellation in the same way as other databases
        // Queries are typically fast, and long-running queries can be interrupted
        Err(DatabaseError::QueryError("Query cancellation not supported for SQLite".to_string()))
    }

    async fn get_tables(&self, conn: &dyn Connection) -> Result<Vec<TableInfo>, DatabaseError> {
        let sqlite_conn = conn
            .as_any()
            .downcast_ref::<SqliteConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;

        let conn_guard = sqlite_conn.conn.lock().await;
        let conn_ref = &*conn_guard;

        let query = r#"
            SELECT name, type
            FROM sqlite_master
            WHERE type IN ('table', 'view')
            AND name NOT LIKE 'sqlite_%'
            ORDER BY name
        "#;

        let mut stmt = conn_ref.prepare(query)
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let tables: Vec<TableInfo> = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let table_type: String = row.get(1)?;
                Ok(TableInfo {
                    schema_name: "main".to_string(),  // SQLite doesn't have schemas
                    table_name: name,
                    table_type,
                    columns: Vec::new(),
                })
            })
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?
            .collect::<Result<_, _>>()
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(tables)
    }

    async fn get_columns(
        &self,
        conn: &dyn Connection,
        table_name: &str,
    ) -> Result<Vec<ColumnInfo>, DatabaseError> {
        let sqlite_conn = conn
            .as_any()
            .downcast_ref::<SqliteConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;

        let conn_guard = sqlite_conn.conn.lock().await;
        let conn_ref = &*conn_guard;

        let query = r#"
            SELECT name, type, dflt_value, pk
            FROM pragma_table_info(?)
            ORDER BY cid
        "#;

        let mut stmt = conn_ref.prepare(query)
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let columns: Vec<ColumnInfo> = stmt
            .query_map([table_name], |row| {
                let name: String = row.get(0)?;
                let data_type: String = row.get(1)?;
                let default: Option<String> = row.get(2)?;
                let pk: i32 = row.get(3)?;

                Ok(ColumnInfo {
                    name,
                    data_type: data_type.clone(),
                    max_length: None,
                    precision: None,
                    scale: None,
                    is_nullable: pk == 0,  // Primary key columns are NOT NULL
                    is_primary_key: pk > 0,
                    is_identity: pk > 0 && data_type == "INTEGER",  // ROWID alias
                    column_default: default,
                    ordinal_position: 0,  // Not available from pragma
                })
            })
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?
            .collect::<Result<_, _>>()
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(columns)
    }

    async fn get_databases(&self, conn: &dyn Connection) -> Result<Vec<String>, DatabaseError> {
        let sqlite_conn = conn
            .as_any()
            .downcast_ref::<SqliteConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;

        let conn_guard = sqlite_conn.conn.lock().await;
        let conn_ref = &*conn_guard;

        // Query for attached databases
        let query = r#"
            SELECT name
            FROM pragma_database_list()
            ORDER BY seq
        "#;

        let mut stmt = conn_ref.prepare(query)
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let databases: Vec<String> = stmt
            .query_map([], |row| {
                Ok(row.get::<_, String>(0)?)
            })
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?
            .collect::<Result<_, _>>()
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        Ok(databases)
    }

    async fn get_schemas(&self, _conn: &dyn Connection) -> Result<Vec<String>, DatabaseError> {
        // SQLite doesn't have schemas like other databases
        // It has "main", "temp", and any attached databases
        Ok(vec!["main".to_string()])
    }
}

impl SqliteDriver {
    /// Convert SQLite type to string
    fn sqlite_type_to_string(sqlite_type: Option<&str>) -> String {
        match sqlite_type {
            Some("INTEGER") => "integer".to_string(),
            Some("REAL") => "real".to_string(),
            Some("TEXT") => "text".to_string(),
            Some("BLOB") => "blob".to_string(),
            Some(t) => t.to_lowercase(),
            None => "any".to_string(),
        }
    }

    /// Extract cell value from row
    fn cell_value_from_row(row: &rusqlite::Row, idx: usize) -> CellValue {
        let idx_usize = idx as usize;

        // Try different types in order of preference
        // SQLite is dynamically typed, so we need to try multiple types

        // Try integer first
        match row.get::<_, i64>(idx_usize) {
            Ok(val) => return CellValue::Int(val),
            Err(_) => {}
        }

        // Try real
        match row.get::<_, f64>(idx_usize) {
            Ok(val) => return CellValue::Float(val),
            Err(_) => {}
        }

        // Try text
        match row.get::<_, String>(idx_usize) {
            Ok(val) => return CellValue::String(val),
            Err(_) => {}
        }

        // Try blob
        match row.get::<_, Vec<u8>>(idx_usize) {
            Ok(val) => return CellValue::Binary(val),
            Err(_) => {}
        }

        CellValue::Null
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_type_to_string() {
        assert_eq!(SqliteDriver::sqlite_type_to_string(Some("INTEGER")), "integer");
        assert_eq!(SqliteDriver::sqlite_type_to_string(Some("REAL")), "real");
        assert_eq!(SqliteDriver::sqlite_type_to_string(Some("TEXT")), "text");
        assert_eq!(SqliteDriver::sqlite_type_to_string(Some("BLOB")), "blob");
        assert_eq!(SqliteDriver::sqlite_type_to_string(None), "any");
    }

    #[test]
    fn test_driver_type() {
        let driver = SqliteDriver::new();
        assert_eq!(driver.database_type(), DatabaseType::Sqlite);
    }

    #[test]
    fn test_config_validation() {
        let mut config = DatabaseConfig::new("Test DB".to_string(), DatabaseType::Sqlite);
        config.database = "/tmp/test.db".to_string();

        assert!(config.validate().is_ok());
    }
}
