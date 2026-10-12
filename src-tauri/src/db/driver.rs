// Database driver abstraction.
//
// Every supported database engine implements `DatabaseDriver`. The generic layers
// (`ConnectionManager`, `QueryEngine`, `SchemaMetadataManager`) only talk to this
// trait and dispatch on `ConnectionConfig::driver`.
//
// Adding a new engine:
//   1. Add a `DriverKind` variant (plus its `as_str` / `FromStr` / `default_port` arms).
//   2. Create a `db/<engine>/` module with a type implementing `DatabaseDriver`.
//   3. Register it in `DriverRegistry::with_builtin_drivers`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use crate::db::config::ConnectionConfig;
use crate::db::error::ConnectionError;
use crate::db::query::{CellValue, ColumnInfo};
use crate::db::schema::{ColumnInfo as SchemaColumnInfo, SchemaInfo};

/// Identifies a database engine. Serialized as a lowercase string
/// (`"mssql"`), which is also the value persisted on spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    /// Microsoft SQL Server (tiberius / TDS)
    #[default]
    Mssql,
    /// MongoDB (official driver; mongo-shell query syntax)
    Mongodb,
}

impl DriverKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DriverKind::Mssql => "mssql",
            DriverKind::Mongodb => "mongodb",
        }
    }

    /// Port used when a connection does not specify one.
    pub fn default_port(&self) -> u16 {
        match self {
            DriverKind::Mssql => 1433,
            DriverKind::Mongodb => 27017,
        }
    }
}

impl fmt::Display for DriverKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DriverKind {
    type Err = ConnectionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mssql" => Ok(DriverKind::Mssql),
            "mongodb" | "mongo" => Ok(DriverKind::Mongodb),
            other => Err(ConnectionError::UnsupportedDriver(other.to_string())),
        }
    }
}

/// A database visible on the server, and whether the current login can use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseEntry {
    pub name: String,
    pub has_access: bool,
}

/// Per-statement execution options, already normalized by the `QueryEngine`.
#[derive(Debug, Clone, Copy)]
pub struct ExecuteOptions<'a> {
    /// Database to run the statement in (`None` = connection default).
    pub database: Option<&'a str>,
    /// Maximum rows to return per result set. Always > 0.
    pub row_limit: usize,
}

/// One tabular result produced by a statement.
#[derive(Debug, Clone, Default)]
pub struct ResultSet {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<CellValue>>,
    /// More rows existed than `ExecuteOptions::row_limit`.
    pub truncated: bool,
}

/// What a statement produced.
#[derive(Debug, Clone)]
pub enum ExecutionOutput {
    /// Zero or more result sets. `row_limit` is the limit that was applied, if any.
    ResultSets {
        sets: Vec<ResultSet>,
        row_limit: Option<usize>,
        /// The native query actually run, when it differs from what the user
        /// wrote (e.g. the aggregation pipeline compiled from SQL on MongoDB)
        generated_query: Option<String>,
    },
    /// A data-modifying statement; carries the number of affected rows.
    RowsAffected(usize),
}

/// Contract every database engine implements.
///
/// Error conventions the `QueryEngine` relies on:
/// - `ConnectionError::QueryError` — the statement itself failed; shown in the result tab.
/// - `ConnectionError::Cancelled` — the statement was interrupted.
/// - anything else — a connection-level failure, surfaced to the caller as an error.
///
/// Cancellation: `execute` is raced against a cancel signal and its future is dropped
/// on cancel. Drivers must make dropping the future stop the work server-side (e.g. by
/// owning the connection inside the future, or issuing a cancel request on drop).
#[async_trait]
pub trait DatabaseDriver: Send + Sync {
    fn kind(&self) -> DriverKind;

    /// Open a throwaway connection to verify the configuration.
    async fn test_connection(&self, config: &ConnectionConfig) -> Result<(), ConnectionError>;

    /// Establish (or reuse) the long-lived connection/pool for `config.id`.
    async fn connect(&self, config: &ConnectionConfig) -> Result<(), ConnectionError>;

    /// Drop the long-lived connection/pool for `connection_id`, if any.
    async fn disconnect(&self, connection_id: &str);

    /// Whether a long-lived connection/pool currently exists for `connection_id`.
    async fn is_connected(&self, connection_id: &str) -> bool;

    /// Whether the long-lived connection for `connection_id` exists and responds.
    async fn is_healthy(&self, connection_id: &str) -> bool;

    /// List databases on the server with an access flag for the current login.
    async fn list_databases(
        &self,
        config: &ConnectionConfig,
    ) -> Result<Vec<DatabaseEntry>, ConnectionError>;

    /// Split editor text into the units sent to the server one at a time.
    fn split_statements(&self, text: &str) -> Vec<String>;

    /// Execute one statement (as returned by `split_statements`).
    async fn execute(
        &self,
        config: &ConnectionConfig,
        statement: &str,
        options: ExecuteOptions<'_>,
    ) -> Result<ExecutionOutput, ConnectionError>;

    /// Fetch schemas, tables/views with columns, relationships and routines.
    async fn fetch_schema(
        &self,
        config: &ConnectionConfig,
        database: &str,
        schema_filter: Option<&str>,
    ) -> Result<SchemaInfo, ConnectionError>;

    /// Fetch the columns of one table or view.
    async fn fetch_table_columns(
        &self,
        config: &ConnectionConfig,
        database: &str,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<SchemaColumnInfo>, ConnectionError>;
}

/// The set of drivers available at runtime, keyed by kind.
pub struct DriverRegistry {
    drivers: HashMap<DriverKind, Arc<dyn DatabaseDriver>>,
}

impl DriverRegistry {
    pub fn empty() -> Self {
        Self {
            drivers: HashMap::new(),
        }
    }

    /// Registry with every driver compiled into the app.
    pub fn with_builtin_drivers() -> Self {
        let mut registry = Self::empty();
        registry.register(Arc::new(crate::db::mssql::MssqlDriver::new()));
        registry.register(Arc::new(crate::db::mongodb::MongoDriver::new()));
        registry
    }

    pub fn register(&mut self, driver: Arc<dyn DatabaseDriver>) {
        self.drivers.insert(driver.kind(), driver);
    }

    pub fn get(&self, kind: DriverKind) -> Result<Arc<dyn DatabaseDriver>, ConnectionError> {
        self.drivers
            .get(&kind)
            .cloned()
            .ok_or_else(|| ConnectionError::UnsupportedDriver(kind.to_string()))
    }

    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn DatabaseDriver>> {
        self.drivers.values()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn driver_kind_round_trips_through_str() {
        assert_eq!("mssql".parse::<DriverKind>().unwrap(), DriverKind::Mssql);
        assert_eq!(" MSSQL ".parse::<DriverKind>().unwrap(), DriverKind::Mssql);
        assert_eq!(DriverKind::Mssql.as_str(), "mssql");
    }

    #[test]
    fn driver_kind_parses_mongodb() {
        assert_eq!(
            "mongodb".parse::<DriverKind>().unwrap(),
            DriverKind::Mongodb
        );
        assert_eq!("mongo".parse::<DriverKind>().unwrap(), DriverKind::Mongodb);
        assert_eq!(DriverKind::Mongodb.default_port(), 27017);
        assert_eq!(
            serde_json::to_string(&DriverKind::Mongodb).unwrap(),
            "\"mongodb\""
        );
    }

    #[test]
    fn driver_kind_rejects_unknown() {
        let err = "oracle".parse::<DriverKind>().unwrap_err();
        assert!(matches!(err, ConnectionError::UnsupportedDriver(ref s) if s == "oracle"));
    }

    #[test]
    fn driver_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&DriverKind::Mssql).unwrap(),
            "\"mssql\""
        );
    }

    #[test]
    fn empty_registry_reports_unsupported_driver() {
        let registry = DriverRegistry::empty();
        assert!(matches!(
            registry.get(DriverKind::Mssql),
            Err(ConnectionError::UnsupportedDriver(_))
        ));
    }

    #[test]
    fn builtin_registry_has_every_driver() {
        let registry = DriverRegistry::with_builtin_drivers();
        for kind in [DriverKind::Mssql, DriverKind::Mongodb] {
            assert_eq!(registry.get(kind).unwrap().kind(), kind);
        }
    }
}
