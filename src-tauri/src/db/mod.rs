// MS-SQL Connection & Query Execution (T015, T016, T017, T024)
// This module handles database connections, query operations, and schema metadata

pub mod connection;
pub mod query;
pub mod schema;
pub mod management;

pub use connection::{
    ConnectionConfig, ConnectionConfigUpdate, ConnectionError, ConnectionInfo,
    MssqlConnectionManager, MssqlPool,
};
pub use query::{CellValue, ColumnInfo, QueryEngine, QueryInfo, QueryResult, QueryStatus};
pub use schema::{
    ColumnInfo as SchemaColumnInfo, RoutineInfo, SchemaInfo, SchemaMetadataManager, TableInfo,
};
