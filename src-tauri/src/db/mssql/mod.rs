// Microsoft SQL Server driver (tiberius / TDS)

mod batch;
mod connection;
pub mod ident;
mod schema;
mod values;

pub use connection::{MssqlClient, MssqlPool};

/// Error type for the in-house bb8 connection manager
/// (see `connection::MssqlConnectionManager` for why it exists).
#[derive(Debug, thiserror::Error)]
pub enum TiberiusError {
    #[error(transparent)]
    Tiberius(#[from] tiberius::error::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<TiberiusError> for ConnectionError {
    fn from(err: TiberiusError) -> Self {
        match err {
            TiberiusError::Tiberius(e) => e.into(),
            TiberiusError::Io(e) => ConnectionError::ConnectionFailed(format!("I/O error: {}", e)),
        }
    }
}

use async_trait::async_trait;
use std::collections::HashSet;
use tiberius::{ColumnType, ToSql};

use crate::db::config::ConnectionConfig;
use crate::db::driver::{
    DatabaseDriver, DatabaseEntry, DriverKind, ExecuteOptions, ExecutionOutput, ResultSet,
};
use crate::db::error::ConnectionError;
use crate::db::schema::{ColumnInfo as SchemaColumnInfo, SchemaInfo};
use batch::{infer_statement_kind, parse_sql_statements, StatementKind};
use connection::{connect_dedicated, MssqlConnectionManager, MssqlPools};

/// SQL Server implementation of `DatabaseDriver`
#[derive(Default)]
pub struct MssqlDriver {
    pools: MssqlPools,
}

impl MssqlDriver {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Map a tiberius error raised while running a statement.
/// A closed/reset connection means the query was cancelled underneath us.
fn statement_error(err: tiberius::error::Error) -> ConnectionError {
    let msg = err.to_string();
    if msg.contains("connection closed") || msg.contains("reset") {
        ConnectionError::Cancelled
    } else {
        ConnectionError::QueryError(msg)
    }
}

/// Run a single-column `name` query on a pooled connection
async fn query_names(
    conn: &mut bb8::PooledConnection<'_, MssqlConnectionManager>,
    query: &str,
) -> Result<Vec<String>, ConnectionError> {
    let rows = conn.simple_query(query).await?.into_first_result().await?;
    Ok(rows
        .iter()
        .filter_map(|row| row.get::<&str, _>(0).map(|s| s.to_string()))
        .collect())
}

#[async_trait]
impl DatabaseDriver for MssqlDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Mssql
    }

    async fn test_connection(&self, config: &ConnectionConfig) -> Result<(), ConnectionError> {
        connect_dedicated(config).await.map(|_client| ())
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<(), ConnectionError> {
        self.pools.get_or_create(config).await.map(|_pool| ())
    }

    async fn disconnect(&self, connection_id: &str) {
        self.pools.remove(connection_id).await;
    }

    async fn is_connected(&self, connection_id: &str) -> bool {
        self.pools.contains(connection_id).await
    }

    async fn is_healthy(&self, connection_id: &str) -> bool {
        match self.pools.get(connection_id).await {
            Some(pool) => pool.get().await.is_ok(),
            None => false,
        }
    }

    /// All online databases, flagged by `HAS_DBACCESS`.
    /// Uses two separate queries and merges in Rust to avoid tiberius type ambiguity
    /// with HAS_DBACCESS() returning typed I32 vs text depending on the protocol path.
    async fn list_databases(
        &self,
        config: &ConnectionConfig,
    ) -> Result<Vec<DatabaseEntry>, ConnectionError> {
        let pool = self.pools.get_or_create(config).await?;
        let mut conn = pool.get().await?;

        // Query 1: all online databases (name only, &str is unambiguous)
        let all_dbs = query_names(
            &mut conn,
            "SELECT name FROM sys.databases WHERE state_desc = 'ONLINE' ORDER BY name",
        )
        .await?;

        // Query 2: only accessible databases
        let accessible: HashSet<String> = query_names(
            &mut conn,
            "SELECT name FROM sys.databases WHERE state_desc = 'ONLINE' AND HAS_DBACCESS(name) = 1 ORDER BY name",
        )
        .await?
        .into_iter()
        .collect();

        Ok(all_dbs
            .into_iter()
            .map(|name| DatabaseEntry {
                has_access: accessible.contains(&name),
                name,
            })
            .collect())
    }

    fn split_statements(&self, text: &str) -> Vec<String> {
        parse_sql_statements(text)
    }

    async fn execute(
        &self,
        config: &ConnectionConfig,
        statement: &str,
        options: ExecuteOptions<'_>,
    ) -> Result<ExecutionOutput, ConnectionError> {
        // Detect final statement behavior (DML vs result set) for scripts such as
        // DECLARE ...; UPDATE ... that should report affected rows.
        let is_dml = infer_statement_kind(statement) == StatementKind::Dml;

        let row_limit = options.row_limit;
        let use_row_limit = !is_dml && row_limit > 0;
        let fetch_limit = row_limit.saturating_add(1);

        let statement_sql = if use_row_limit {
            format!(
                "SET ROWCOUNT {}; {}; SET ROWCOUNT 0;",
                fetch_limit, statement
            )
        } else {
            statement.to_string()
        };

        // Build the full query with optional USE database — database identifier is validated + bracket-quoted
        let full_query = match options.database {
            Some(db) => {
                let use_db =
                    ident::resolve_database_name(db).map_err(ConnectionError::QueryError)?;
                format!("USE {use_db}; {statement_sql}")
            }
            None => statement_sql,
        };

        // A dedicated connection (not from the pool): the QueryEngine cancels by
        // dropping this future, which drops the client and closes the TCP stream.
        let mut conn = connect_dedicated(config).await?;

        if is_dml {
            let params: &[&dyn ToSql] = &[];
            let execute_result = conn
                .execute(&full_query, params)
                .await
                .map_err(statement_error)?;
            let affected_rows =
                execute_result.rows_affected().iter().copied().sum::<u64>() as usize;
            return Ok(ExecutionOutput::RowsAffected(affected_rows));
        }

        let all_results = conn
            .simple_query(&full_query)
            .await
            .map_err(statement_error)?
            .into_results()
            .await
            .map_err(statement_error)?;

        let sets = all_results
            .iter()
            .map(|rows| {
                let Some(first) = rows.first() else {
                    return ResultSet::default();
                };
                let columns = first.columns().iter().map(values::column_info).collect();
                let col_types: Vec<ColumnType> =
                    first.columns().iter().map(|c| c.column_type()).collect();

                let mut converted: Vec<Vec<_>> = rows
                    .iter()
                    .map(|row| {
                        col_types
                            .iter()
                            .enumerate()
                            .map(|(idx, col_type)| values::cell_from_row(row, idx, col_type))
                            .collect()
                    })
                    .collect();

                let truncated = use_row_limit && converted.len() > row_limit;
                if truncated {
                    converted.truncate(row_limit);
                }
                ResultSet {
                    columns,
                    rows: converted,
                    truncated,
                }
            })
            .collect();

        Ok(ExecutionOutput::ResultSets {
            sets,
            row_limit: use_row_limit.then_some(row_limit),
            generated_query: None,
        })
    }

    async fn fetch_schema(
        &self,
        config: &ConnectionConfig,
        database: &str,
        schema_filter: Option<&str>,
    ) -> Result<SchemaInfo, ConnectionError> {
        schema::fetch_schema(config, database, schema_filter).await
    }

    async fn fetch_table_columns(
        &self,
        config: &ConnectionConfig,
        database: &str,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<SchemaColumnInfo>, ConnectionError> {
        schema::fetch_table_columns(config, database, schema_name, table_name).await
    }
}
