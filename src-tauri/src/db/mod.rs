// Database Module (Phase 1 & 2 - Extensible Architecture)
// Handles database connections, query operations, and schema metadata
// Now supports extensible database drivers

// Legacy modules (maintained for backward compatibility)
pub mod connection;
pub mod management;
pub mod query;
pub mod schema;

// New extensible architecture modules
pub mod drivers;
pub mod managers;
pub mod postgres_manager;
pub mod registry;
pub mod sqlite_manager;
pub mod traits;

// ============================================================================
// Legacy Re-exports (Phase 2 - Keep backward compatibility)
// ============================================================================

pub use connection::{
    ConnectionConfig, ConnectionConfigUpdate, ConnectionError, ConnectionInfo,
    MssqlConnectionManager, MssqlPool,
};
pub use query::{QueryEngine, QueryInfo, QueryStatus};
pub use schema::{
    ColumnInfo as SchemaColumnInfo, RoutineInfo, SchemaInfo, SchemaMetadataManager,
    TableInfo as SchemaTableInfo,
};

// ============================================================================
// New Extensible Types (Phase 1)
// ============================================================================

pub use drivers::{MssqlDriver, PostgresDriver, SqliteDriver};
pub use managers::UnifiedConnectionManager;
pub use postgres_manager::{PostgresConfig, PostgresConnectionManager};
pub use registry::DriverRegistry;
pub use sqlite_manager::{SqliteConfig, SqliteConnectionManager};
pub use traits::{
    CellValue, ColumnInfo, Connection, DatabaseConfig, DatabaseDriver, DatabaseError, DatabaseType,
    QueryResult, TableInfo,
};

// ============================================================================
// Type Aliases for Clarity
// ============================================================================
