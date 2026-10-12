// Database connections, query execution and schema metadata (T015, T016, T017, T024)
//
// Layout:
// - driver.rs  — `DatabaseDriver` trait, `DriverKind`, `DriverRegistry`
// - config.rs / error.rs — driver-neutral connection config and errors
// - manager.rs — `ConnectionManager`: config registry, dispatches to drivers
// - query.rs   — `QueryEngine`: batching, cancellation, result shaping
// - schema.rs  — schema model + `SchemaMetadataManager` (caching)
// - mssql/     — SQL Server driver (tiberius)
// - postgres/  — PostgreSQL driver (tokio-postgres)
// - mongodb/   — MongoDB driver (mongo-shell syntax, official driver)

// Logging macros using println for simplicity (no trailing semicolon for use in match arms).
// Defined before the submodules so they are in scope there.
macro_rules! log_info {
    ($($arg:tt)*) => {{ println!("[INFO] {}", format!($($arg)*)) }};
}
macro_rules! log_warn {
    ($($arg:tt)*) => {{ println!("[WARN] {}", format!($($arg)*)) }};
}

pub mod config;
pub mod driver;
pub mod error;
pub mod management;
pub mod manager;
pub mod mongodb;
pub mod mssql;
pub mod postgres;
pub mod query;
pub mod schema;

pub use config::{ConnectionConfig, ConnectionConfigUpdate, ConnectionInfo};
pub use driver::{DatabaseDriver, DatabaseEntry, DriverKind, DriverRegistry};
pub use error::ConnectionError;
pub use manager::ConnectionManager;
pub use query::{CellValue, ColumnInfo, QueryEngine, QueryInfo, QueryResult, QueryStatus};
pub use schema::{
    ColumnInfo as SchemaColumnInfo, RelationshipInfo as SchemaRelationshipInfo, RoutineInfo,
    SchemaInfo, SchemaMetadataManager, TableInfo,
};
