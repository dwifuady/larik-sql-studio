// Database Driver Traits (Phase 1 - Extensible Architecture)
// Defines the core abstraction for supporting multiple database types

use serde::{Deserialize, Serialize};

/// Supported database types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DatabaseType {
    Sqlite,
    Mssql,
    Postgresql,
    Mysql,
}

impl DatabaseType {
    /// Display name for UI
    pub fn display_name(&self) -> &'static str {
        match self {
            DatabaseType::Sqlite => "SQLite",
            DatabaseType::Mssql => "Microsoft SQL Server",
            DatabaseType::Postgresql => "PostgreSQL",
            DatabaseType::Mysql => "MySQL",
        }
    }

    /// Default port for the database type
    pub fn default_port(&self) -> u16 {
        match self {
            DatabaseType::Sqlite => 0, // File-based, no port
            DatabaseType::Mssql => 1433,
            DatabaseType::Postgresql => 5432,
            DatabaseType::Mysql => 3306,
        }
    }
}

/// Common database error type
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),

    #[error("Driver not found for database type: {0:?}")]
    DriverNotFound(DatabaseType),

    #[error("Query execution error: {0}")]
    QueryError(String),

    #[error("Configuration error: {0}")]
    InvalidConfig(String),

    #[error("Invalid connection type")]
    InvalidConnection,

    #[error("Query not found: {0}")]
    QueryNotFound(String),

    #[error("Timeout error")]
    Timeout,

    #[error("Pool error: {0}")]
    PoolError(String),

    #[error("Schema error: {0}")]
    SchemaError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Column information from query results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub max_length: Option<i32>,
    pub precision: Option<i32>,
    pub scale: Option<i32>,
    pub is_nullable: bool,
    pub is_primary_key: bool,
    pub is_identity: bool,
    pub column_default: Option<String>,
    pub ordinal_position: i32,
}

/// Cell value in a result set
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
    pub is_selection: bool,
    pub statement_index: Option<usize>,
    pub statement_text: Option<String>,
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
        let mut result = Self::new(query_id);
        result.error = Some(error);
        result.is_complete = true;
        result
    }
}

/// Table information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub schema_name: String,
    pub table_name: String,
    pub table_type: String,
    pub columns: Vec<ColumnInfo>,
}

/// Connection trait - all database connections must implement this
#[async_trait::async_trait]
pub trait Connection: Send + Sync {
    /// Get the connection ID
    fn connection_id(&self) -> &str;

    /// Test if the connection is alive
    async fn is_alive(&self) -> bool;

    /// Allow downcasting for driver-specific operations
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Database driver trait - all database drivers must implement this
#[async_trait::async_trait]
pub trait DatabaseDriver: Send + Sync {
    // --- Metadata ---
    /// Get the database type this driver supports
    fn database_type(&self) -> DatabaseType;

    /// Get the display name for this driver
    fn driver_name(&self) -> &'static str {
        self.database_type().display_name()
    }

    // --- Connection Management ---
    /// Test a connection configuration without creating a persistent connection
    async fn test_connection(&self, config: &DatabaseConfig) -> Result<bool, DatabaseError>;

    /// Create a new connection from configuration
    async fn connect(&self, config: &DatabaseConfig) -> Result<Box<dyn Connection>, DatabaseError>;

    // --- Query Execution ---
    /// Execute a SQL query and return results
    async fn execute_query(
        &self,
        conn: &dyn Connection,
        sql: &str,
        query_id: String,
        database: Option<&str>,
    ) -> Result<QueryResult, DatabaseError>;

    /// Cancel a running query (if supported)
    async fn cancel_query(&self, query_id: &str) -> Result<(), DatabaseError>;

    // --- Schema Metadata ---
    /// Get list of tables/views
    async fn get_tables(&self, conn: &dyn Connection) -> Result<Vec<TableInfo>, DatabaseError>;

    /// Get columns for a specific table
    async fn get_columns(
        &self,
        conn: &dyn Connection,
        table_name: &str,
    ) -> Result<Vec<ColumnInfo>, DatabaseError>;

    /// Get list of databases (catalogs)
    async fn get_databases(&self, conn: &dyn Connection) -> Result<Vec<String>, DatabaseError>;

    /// Get list of schemas
    async fn get_schemas(&self, conn: &dyn Connection) -> Result<Vec<String>, DatabaseError>;
}

/// Unified database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub id: String,
    pub name: String,
    pub database_type: DatabaseType,
    pub space_id: Option<String>,

    // Common connection fields
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: String,
    pub username: Option<String>,
    #[serde(skip_serializing)]
    pub password: String,

    // MS-SQL specific
    pub mssql_encrypt: Option<bool>,
    pub mssql_trust_cert: Option<bool>,

    // PostgreSQL specific
    pub postgres_sslmode: Option<String>, // "disable", "require", "verify-ca", "verify-full"

    // MySQL specific
    pub mysql_ssl_enabled: Option<bool>,
}

impl DatabaseConfig {
    pub fn new(name: String, database_type: DatabaseType) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            database_type,
            space_id: None,
            host: None,
            port: None,
            database: String::new(),
            username: None,
            password: String::new(),
            mssql_encrypt: None,
            mssql_trust_cert: None,
            postgres_sslmode: None,
            mysql_ssl_enabled: None,
        }
    }

    pub fn validate(&self) -> Result<(), DatabaseError> {
        match self.database_type {
            DatabaseType::Sqlite => {
                if self.database.is_empty() {
                    return Err(DatabaseError::InvalidConfig(
                        "SQLite database path is required".to_string(),
                    ));
                }
            }
            DatabaseType::Mssql | DatabaseType::Postgresql | DatabaseType::Mysql => {
                if self.host.is_none() || self.host.as_ref().map(|h| h.is_empty()).unwrap_or(true) {
                    return Err(DatabaseError::InvalidConfig("Host is required".to_string()));
                }
                if self.username.is_none()
                    || self.username.as_ref().map(|u| u.is_empty()).unwrap_or(true)
                {
                    return Err(DatabaseError::InvalidConfig("Username is required".to_string()));
                }
                if self.database.is_empty() {
                    return Err(DatabaseError::InvalidConfig("Database name is required".to_string()));
                }
            }
        }
        Ok(())
    }

    pub fn get_port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.database_type.default_port())
    }
}
