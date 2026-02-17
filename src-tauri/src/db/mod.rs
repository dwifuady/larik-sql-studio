// Database Module (Phase 1 & 2 - Extensible Architecture)
// Handles database connections, query operations, and schema metadata
// Now supports extensible database drivers

// Legacy modules (maintained for backward compatibility)
pub mod connection;
pub mod query;
pub mod schema;
pub mod management;

// New extensible architecture modules
pub mod traits;
pub mod registry;
pub mod drivers;

// ============================================================================
// Legacy Re-exports (Phase 2 - Keep backward compatibility)
// ============================================================================

pub use connection::{
    ConnectionConfig, ConnectionConfigUpdate, ConnectionError, ConnectionInfo,
    MssqlConnectionManager, MssqlPool,
};
pub use query::{CellValue, ColumnInfo as QueryColumnInfo, QueryEngine, QueryInfo, QueryResult, QueryStatus};
pub use schema::{
    ColumnInfo as SchemaColumnInfo, RoutineInfo, SchemaInfo, SchemaMetadataManager, TableInfo as SchemaTableInfo,
};

// ============================================================================
// New Extensible Types (Phase 1)
// ============================================================================

pub use traits::{
    DatabaseDriver, DatabaseType, DatabaseConfig, DatabaseError, Connection,
    ColumnInfo as UnifiedColumnInfo, CellValue as UnifiedCellValue, QueryResult as UnifiedQueryResult, TableInfo as UnifiedTableInfo,
};
pub use registry::DriverRegistry;
pub use drivers::{MssqlDriver, SqliteDriver};

// ============================================================================
// Type Aliases for Clarity
// ============================================================================
