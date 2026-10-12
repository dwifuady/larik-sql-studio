// PostgreSQL driver (tokio-postgres / extended + simple query protocols)

mod batch;
mod connection;
pub mod ident;
mod schema;
mod values;

use async_trait::async_trait;
use tokio_postgres::types::Type;

use crate::db::config::ConnectionConfig;
use crate::db::driver::{
    DatabaseDriver, DatabaseEntry, DriverKind, ExecuteOptions, ExecutionOutput, ResultSet,
};
use crate::db::error::ConnectionError;
use crate::db::query::{CellValue, ColumnInfo};
use crate::db::schema::{ColumnInfo as SchemaColumnInfo, SchemaInfo};
use batch::{classify_statement, parse_sql_statements, StatementKind};
use connection::{connect_dedicated, is_healthy, statement_error, PgClients};

/// PostgreSQL implementation of `DatabaseDriver`
#[derive(Default)]
pub struct PostgresDriver {
    clients: PgClients,
}

impl PostgresDriver {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DatabaseDriver for PostgresDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Postgres
    }

    async fn test_connection(&self, config: &ConnectionConfig) -> Result<(), ConnectionError> {
        connect_dedicated(config, None).await.map(|_| ())
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<(), ConnectionError> {
        self.clients.get_or_create(config).await.map(|_| ())
    }

    async fn disconnect(&self, connection_id: &str) {
        self.clients.remove(connection_id).await;
    }

    async fn is_connected(&self, connection_id: &str) -> bool {
        self.clients.contains(connection_id).await
    }

    async fn is_healthy(&self, connection_id: &str) -> bool {
        match self.clients.get(connection_id).await {
            Some(client) => is_healthy(&client).await,
            None => false,
        }
    }

    /// All connectable databases, flagged by `CONNECT` privilege — the
    /// PostgreSQL analog of SQL Server's `HAS_DBACCESS()`.
    async fn list_databases(
        &self,
        config: &ConnectionConfig,
    ) -> Result<Vec<DatabaseEntry>, ConnectionError> {
        let conn = connect_dedicated(config, None).await?;
        let rows = conn
            .query(
                "SELECT datname, has_database_privilege(current_user, datname, 'CONNECT') \
                 FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname",
                &[],
            )
            .await
            .map_err(statement_error)?;

        Ok(rows
            .iter()
            .map(|r| DatabaseEntry {
                name: r.get(0),
                has_access: r.get(1),
            })
            .collect())
    }

    fn split_statements(&self, text: &str) -> Vec<String> {
        parse_sql_statements(text)
    }

    /// Note on cancellation: every statement runs on a dedicated connection.
    /// The QueryEngine cancels by dropping this future, which drops the client
    /// and closes the TCP stream — the server cancels the statement when the
    /// socket closes.
    async fn execute(
        &self,
        config: &ConnectionConfig,
        statement: &str,
        options: ExecuteOptions<'_>,
    ) -> Result<ExecutionOutput, ConnectionError> {
        let conn = connect_dedicated(config, options.database).await?;

        match classify_statement(statement) {
            StatementKind::SimpleProtocol => execute_simple(&conn, statement).await,
            StatementKind::ExtendedProtocol => execute_extended(&conn, statement, options).await,
        }
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

/// Run a statement through the simple query protocol (utility commands that
/// cannot run inside the extended protocol's implicit transaction).
/// Values arrive as text; a rowless statement reports affected rows.
async fn execute_simple(
    conn: &tokio_postgres::Client,
    statement: &str,
) -> Result<ExecutionOutput, ConnectionError> {
    let messages = conn
        .simple_query(statement)
        .await
        .map_err(statement_error)?;

    let mut columns: Vec<ColumnInfo> = Vec::new();
    let mut rows: Vec<Vec<CellValue>> = Vec::new();
    let mut affected: usize = 0;
    for message in messages {
        match message {
            tokio_postgres::SimpleQueryMessage::RowDescription(cols) => {
                columns = cols
                    .iter()
                    .map(|c| ColumnInfo {
                        name: c.name().to_string(),
                        data_type: "text".to_string(),
                        nullable: true,
                    })
                    .collect();
            }
            tokio_postgres::SimpleQueryMessage::Row(row) => {
                rows.push(
                    (0..columns.len())
                        .map(|idx| match row.get(idx) {
                            Some(v) => CellValue::String(v.to_string()),
                            None => CellValue::Null,
                        })
                        .collect(),
                );
            }
            tokio_postgres::SimpleQueryMessage::CommandComplete(n) => affected = n as usize,
            // CopyOut/CopyIn/CopyBoth responses carry no result rows here.
            _ => {}
        }
    }

    if rows.is_empty() {
        Ok(ExecutionOutput::RowsAffected(affected))
    } else {
        Ok(ExecutionOutput::ResultSets {
            sets: vec![ResultSet {
                columns,
                rows,
                truncated: false,
            }],
            row_limit: None,
            generated_query: None,
        })
    }
}

/// Run a statement through prepare + query/execute.
/// - No result columns → DML: report affected rows.
/// - Result columns → stream rows, stopping one past the row limit (the
///   remaining rows are cancelled by dropping the connection).
async fn execute_extended(
    conn: &tokio_postgres::Client,
    statement: &str,
    options: ExecuteOptions<'_>,
) -> Result<ExecutionOutput, ConnectionError> {
    use futures::StreamExt;

    let prepared = conn.prepare(statement).await.map_err(statement_error)?;
    let columns: Vec<ColumnInfo> = prepared.columns().iter().map(values::column_info).collect();
    let types: Vec<Type> = prepared
        .columns()
        .iter()
        .map(|c| c.type_().clone())
        .collect();

    if columns.is_empty() {
        let affected = conn
            .execute(&prepared, &[])
            .await
            .map_err(statement_error)?;
        return Ok(ExecutionOutput::RowsAffected(affected as usize));
    }

    let stream = conn
        .query_raw(&prepared, Vec::<i32>::new())
        .await
        .map_err(statement_error)?;
    let row_limit = options.row_limit;
    futures::pin_mut!(stream);

    let mut rows: Vec<Vec<CellValue>> = Vec::new();
    let mut truncated = false;
    while let Some(item) = stream.next().await {
        if rows.len() == row_limit {
            truncated = true;
            break;
        }
        let row = item.map_err(statement_error)?;
        rows.push(
            (0..types.len())
                .map(|idx| values::cell_from_row(&row, idx, &types[idx]))
                .collect(),
        );
    }
    // Dropping `stream` and `conn` closes the socket and cancels the server.

    Ok(ExecutionOutput::ResultSets {
        sets: vec![ResultSet {
            columns,
            rows,
            truncated,
        }],
        row_limit: Some(row_limit),
        generated_query: None,
    })
}

/// End-to-end tests against a real server. Run with:
///   LARIK_TEST_POSTGRES_URL=postgres://postgres:postgres@localhost:5432/postgres cargo test -- --ignored postgres
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod live_tests {
    use super::*;
    use crate::db::query::CellValue;

    fn live_config() -> Option<ConnectionConfig> {
        let url = std::env::var("LARIK_TEST_POSTGRES_URL").ok()?;
        let (host, port, database, username, password) = parse_url(&url);
        let mut config = ConnectionConfig::new(
            DriverKind::Postgres,
            "live".to_string(),
            host,
            port,
            database,
            username,
            password,
        );
        config.id = format!("live-{}", uuid::Uuid::new_v4());
        Some(config)
    }

    /// Minimal postgres://user:pass@host:port/db parsing for the test env var.
    fn parse_url(url: &str) -> (String, u16, String, String, String) {
        let rest = url
            .strip_prefix("postgres://")
            .or_else(|| url.strip_prefix("postgresql://"))
            .expect("LARIK_TEST_POSTGRES_URL must be postgres://user:pass@host:port/db");
        let (authority, database) = match rest.split_once('/') {
            Some((a, d)) => (a, d.to_string()),
            None => (rest, "postgres".to_string()),
        };
        let (userinfo, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (u.to_string(), h.to_string()),
            None => (String::new(), authority.to_string()),
        };
        let (username, password) = match userinfo.split_once(':') {
            Some((u, p)) => (u.to_string(), p.to_string()),
            None => (userinfo, String::new()),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().unwrap_or(5432)),
            None => (hostport, 5432),
        };
        (host, port, database, username, password)
    }

    async fn run(
        config: &ConnectionConfig,
        statement: &str,
        database: Option<&str>,
    ) -> Result<ExecutionOutput, ConnectionError> {
        let driver = PostgresDriver::new();
        driver
            .execute(
                config,
                statement,
                ExecuteOptions {
                    database,
                    row_limit: 10_000,
                },
            )
            .await
    }

    fn url_available() -> bool {
        std::env::var("LARIK_TEST_POSTGRES_URL").is_ok()
    }
    #[ignore]
    #[tokio::test]
    async fn connects_and_lists_databases() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let driver = PostgresDriver::new();
        driver.test_connection(&config).await.unwrap();
        driver.connect(&config).await.unwrap();
        assert!(driver.is_connected(&config.id).await);
        assert!(driver.is_healthy(&config.id).await);

        let dbs = driver.list_databases(&config).await.unwrap();
        assert!(!dbs.is_empty());
        // The configured database is always accessible by definition.
        assert!(dbs.iter().any(|d| d.name == config.database && d.has_access));
    }
    #[ignore]
    #[tokio::test]
    async fn type_round_trip() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let sql = r#"
            SELECT
                42::int AS i,
                9007199254740993::bigint AS l,
                3.25::float8 AS f,
                12.345::numeric AS n,
                true AS b,
                'héllo' AS t,
                '2024-03-05'::date AS d,
                '2024-03-05T10:11:12Z'::timestamptz AS ts,
                '2024-03-05 10:11:12'::timestamp AS ts2,
                'a1b2c3d4-0000-4000-8000-000000000000'::uuid AS u,
                '{"k": [1, 2]}'::jsonb AS j,
                '\x00ff10'::bytea AS bin,
                ARRAY[1, 2, 3] AS ints,
                ARRAY['a', 'b c'] AS texts,
                NULL::int AS nil
        "#;
        match run(&config, sql, None).await.unwrap() {
            ExecutionOutput::ResultSets { sets, .. } => {
                let rows = &sets[0].rows;
                assert_eq!(rows.len(), 1);
                let r = &rows[0];
                assert_eq!(r[0], CellValue::Int(42));
                assert_eq!(r[1], CellValue::Int(9007199254740993));
                assert_eq!(r[2], CellValue::Float(3.25));
                assert_eq!(r[3], CellValue::Float(12.345));
                assert_eq!(r[4], CellValue::Bool(true));
                assert_eq!(r[5], CellValue::String("héllo".to_string()));
                assert_eq!(r[6], CellValue::DateTime("2024-03-05".to_string()));
                assert!(matches!(r[7], CellValue::DateTime(_)));
                assert_eq!(
                    r[8],
                    CellValue::DateTime("2024-03-05 10:11:12".to_string())
                );
                assert_eq!(
                    r[9],
                    CellValue::String("a1b2c3d4-0000-4000-8000-000000000000".to_string())
                );
                assert_eq!(r[10], CellValue::String("{\"k\":[1,2]}".to_string()));
                assert!(matches!(r[11], CellValue::Binary(_)));
                assert_eq!(r[12], CellValue::String("{1,2,3}".to_string()));
                assert_eq!(r[13], CellValue::String("{a,\"b c\"}".to_string()));
                assert_eq!(r[14], CellValue::Null);
            }
            other => panic!("expected result sets, got {other:?}"),
        }
    }
    #[ignore]
    #[tokio::test]
    async fn dml_reports_rows_affected_and_returning_gives_rows() {
        if !url_available() {
            return;
        }
        // Real table: temp tables are session-scoped and every statement uses
        // its own dedicated connection.
        let config = live_config().unwrap();
        let table = format!("larik_pg_test_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        run(&config, &format!("DROP TABLE IF EXISTS {table}"), None).await.unwrap();
        run(&config, &format!("CREATE TABLE {table} (id int PRIMARY KEY, v text)"), None).await.unwrap();

        let result = run(
            &config,
            &format!("INSERT INTO {table} SELECT g, g::text FROM generate_series(1, 3) g"),
            None,
        )
        .await;
        match result.unwrap() {
            ExecutionOutput::RowsAffected(n) => assert_eq!(n, 3),
            other => panic!("expected rows affected, got {other:?}"),
        }

        match run(&config, &format!("UPDATE {table} SET v = v RETURNING id"), None)
            .await
            .unwrap()
        {
            ExecutionOutput::ResultSets { sets, .. } => assert_eq!(sets[0].rows.len(), 3),
            other => panic!("expected returning rows, got {other:?}"),
        }

        match run(&config, &format!("DELETE FROM {table}"), None).await.unwrap() {
            ExecutionOutput::RowsAffected(n) => assert_eq!(n, 3),
            other => panic!("expected rows affected, got {other:?}"),
        }

        run(&config, &format!("DROP TABLE {table}"), None).await.unwrap();
    }
    #[ignore]
    #[tokio::test]
    async fn row_limit_truncates() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let driver = PostgresDriver::new();
        let output = driver
            .execute(
                &config,
                "SELECT g FROM generate_series(1, 50) g",
                ExecuteOptions {
                    database: None,
                    row_limit: 10,
                },
            )
            .await
            .unwrap();
        match output {
            ExecutionOutput::ResultSets { sets, .. } => {
                assert_eq!(sets[0].rows.len(), 10);
                assert!(sets[0].truncated);
            }
            other => panic!("expected result sets, got {other:?}"),
        }
    }
    #[ignore]
    #[tokio::test]
    async fn schema_fetch_lists_tables_and_relationships() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let a = format!("larik_pg_a_{suffix}");
        let b = format!("larik_pg_b_{suffix}");
        run(&config, &format!("DROP TABLE IF EXISTS {b}, {a}"), None)
            .await
            .unwrap();
        run(&config, &format!("CREATE TABLE {a} (id int PRIMARY KEY)"), None)
            .await
            .unwrap();
        run(
            &config,
            &format!("CREATE TABLE {b} (id int PRIMARY KEY, a_id int REFERENCES {a}(id))"),
            None,
        )
        .await
        .unwrap();

        let driver = PostgresDriver::new();
        let schema = driver
            .fetch_schema(&config, &config.database, None)
            .await
            .unwrap();
        let cols = driver
            .fetch_table_columns(&config, &config.database, "public", &b)
            .await
            .unwrap();
        run(&config, &format!("DROP TABLE {b}, {a}"), None).await.unwrap();

        assert!(schema
            .tables
            .iter()
            .any(|t| t.table_name == b && t.schema_name == "public"));
        assert!(schema.relationships.iter().any(|r| r.source_table_name == b
            && r.source_column_name == "a_id"
            && r.target_table_name == a));
        assert_eq!(cols.len(), 2);
        assert!(cols.iter().any(|c| c.name == "id" && c.is_primary_key && !c.is_nullable));
        assert!(cols.iter().any(|c| c.name == "a_id" && c.is_nullable));
    }

    #[tokio::test]
    async fn statement_splitting_handles_dollar_quotes() {
        let driver = PostgresDriver::new();
        let statements = driver.split_statements("SELECT 1; SELECT $$a;b$$;");
        assert_eq!(statements.len(), 2);
    }
}
