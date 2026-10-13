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
use connection::{
    connect_dedicated, is_healthy, statement_error, CancelOnDrop, PgClients, TlsMode,
};

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

    /// Statements sharing session state (transactions, `SET`, temp tables)
    /// are kept together as one unit; see `batch::split_statements`.
    fn split_statements(&self, text: &str) -> Vec<String> {
        batch::split_statements(text)
    }

    /// Runs one unit on a dedicated connection. A unit holding several
    /// statements (a session script) runs them in order on that connection.
    ///
    /// Cancellation: the QueryEngine cancels by dropping this future. Dropping
    /// the client alone would leave the statement running server-side
    /// (tokio-postgres keeps the connection task alive until in-flight
    /// requests finish), so a `CancelOnDrop` guard sends a CancelRequest.
    async fn execute(
        &self,
        config: &ConnectionConfig,
        statement: &str,
        options: ExecuteOptions<'_>,
    ) -> Result<ExecutionOutput, ConnectionError> {
        let conn = connect_dedicated(config, options.database).await?;
        let guard = CancelOnDrop::new(&conn, TlsMode::from_config(config));

        let mut statements = parse_sql_statements(statement);
        if statements.is_empty() {
            statements.push(statement.to_string());
        }
        let result = run_script(&conn, &statements, options.row_limit).await;

        // A truncated result stopped reading early: cancel the remainder
        // instead of letting the server finish and stream it to nobody.
        let truncated = matches!(
            &result,
            Ok(ExecutionOutput::ResultSets { sets, .. }) if sets.iter().any(|s| s.truncated)
        );
        if truncated {
            drop(guard);
        } else {
            guard.disarm();
        }
        result
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

/// Run statements in order on one connection, collecting every result set.
/// Rowless statements contribute their affected-row counts.
async fn run_script(
    conn: &tokio_postgres::Client,
    statements: &[String],
    row_limit: usize,
) -> Result<ExecutionOutput, ConnectionError> {
    let mut sets: Vec<ResultSet> = Vec::new();
    let mut affected = 0usize;
    for (index, statement) in statements.iter().enumerate() {
        let outcome = match classify_statement(statement) {
            StatementKind::SimpleProtocol => execute_simple(conn, statement, row_limit).await,
            StatementKind::ExtendedProtocol => execute_extended(conn, statement, row_limit).await,
        };
        let (statement_sets, statement_affected) = match outcome {
            Ok(out) => out,
            // Say which statement of a script failed.
            Err(ConnectionError::QueryError(msg)) if statements.len() > 1 => {
                return Err(ConnectionError::QueryError(format!(
                    "Statement {}: {msg}",
                    index + 1
                )));
            }
            Err(e) => return Err(e),
        };
        sets.extend(statement_sets);
        affected += statement_affected;
    }

    if sets.is_empty() {
        Ok(ExecutionOutput::RowsAffected(affected))
    } else {
        Ok(ExecutionOutput::ResultSets {
            sets,
            row_limit: Some(row_limit),
            generated_query: None,
        })
    }
}

/// Run a statement through the simple query protocol (utility commands that
/// cannot run inside the extended protocol's implicit transaction).
/// Values arrive as text. Rows past the limit are read and discarded so the
/// session stays usable for the statements that follow.
async fn execute_simple(
    conn: &tokio_postgres::Client,
    statement: &str,
    row_limit: usize,
) -> Result<(Vec<ResultSet>, usize), ConnectionError> {
    use futures::StreamExt;
    use tokio_postgres::SimpleQueryMessage;

    let stream = conn
        .simple_query_raw(statement)
        .await
        .map_err(statement_error)?;
    futures::pin_mut!(stream);

    let mut sets: Vec<ResultSet> = Vec::new();
    let mut open: Option<ResultSet> = None;
    let mut affected = 0usize;
    while let Some(message) = stream.next().await {
        match message.map_err(statement_error)? {
            SimpleQueryMessage::RowDescription(cols) => {
                open = Some(ResultSet {
                    columns: cols
                        .iter()
                        .map(|c| ColumnInfo {
                            name: c.name().to_string(),
                            data_type: "text".to_string(),
                            nullable: true,
                        })
                        .collect(),
                    rows: Vec::new(),
                    truncated: false,
                });
            }
            SimpleQueryMessage::Row(row) => {
                if let Some(set) = open.as_mut() {
                    if set.rows.len() >= row_limit {
                        set.truncated = true;
                        continue;
                    }
                    set.rows.push(
                        (0..set.columns.len())
                            .map(|idx| match row.get(idx) {
                                Some(v) => CellValue::String(v.to_string()),
                                None => CellValue::Null,
                            })
                            .collect(),
                    );
                }
            }
            SimpleQueryMessage::CommandComplete(n) => match open.take() {
                Some(set) => sets.push(set),
                None => affected += n as usize,
            },
            _ => {}
        }
    }
    Ok((sets, affected))
}

/// Run a statement through prepare + query/execute.
/// - No result columns → DML: report affected rows.
/// - Result columns → stream rows, stopping one past the row limit (the
///   caller cancels the remainder server-side).
async fn execute_extended(
    conn: &tokio_postgres::Client,
    statement: &str,
    row_limit: usize,
) -> Result<(Vec<ResultSet>, usize), ConnectionError> {
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
        return Ok((Vec::new(), affected as usize));
    }

    let stream = conn
        .query_raw(&prepared, Vec::<i32>::new())
        .await
        .map_err(statement_error)?;
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
            types
                .iter()
                .enumerate()
                .map(|(idx, ty)| values::cell_from_row(&row, idx, ty))
                .collect(),
        );
    }

    Ok((
        vec![ResultSet {
            columns,
            rows,
            truncated,
        }],
        0,
    ))
}

/// End-to-end tests against a real server. Run with:
///   LARIK_TEST_POSTGRES_URL=postgres://postgres:postgres@localhost:5432/postgres cargo test -- --ignored postgres
/// Connections are plaintext unless LARIK_TEST_POSTGRES_TLS is set (e.g. the
/// stock `postgres` Docker image has SSL off).
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
        // Set LARIK_TEST_POSTGRES_TLS=1 when the test server accepts SSL.
        config.encrypt = std::env::var("LARIK_TEST_POSTGRES_TLS").is_ok();
        config.trust_certificate = true;
        Some(config)
    }

    #[ignore]
    #[tokio::test]
    async fn encrypt_never_downgrades_to_plaintext() {
        if !url_available() || std::env::var("LARIK_TEST_POSTGRES_TLS").is_ok() {
            return;
        }
        // Against a server without SSL, Encrypt must fail instead of
        // quietly connecting in plaintext.
        let mut config = live_config().unwrap();
        config.encrypt = true;
        let err = PostgresDriver::new()
            .test_connection(&config)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ConnectionError::ConnectionFailed(_)),
            "{err:?}"
        );
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
        assert!(dbs
            .iter()
            .any(|d| d.name == config.database && d.has_access));
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
                assert_eq!(r[8], CellValue::DateTime("2024-03-05 10:11:12".to_string()));
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
        let table = format!(
            "larik_pg_test_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        run(&config, &format!("DROP TABLE IF EXISTS {table}"), None)
            .await
            .unwrap();
        run(
            &config,
            &format!("CREATE TABLE {table} (id int PRIMARY KEY, v text)"),
            None,
        )
        .await
        .unwrap();

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

        match run(
            &config,
            &format!("UPDATE {table} SET v = v RETURNING id"),
            None,
        )
        .await
        .unwrap()
        {
            ExecutionOutput::ResultSets { sets, .. } => assert_eq!(sets[0].rows.len(), 3),
            other => panic!("expected returning rows, got {other:?}"),
        }

        match run(&config, &format!("DELETE FROM {table}"), None)
            .await
            .unwrap()
        {
            ExecutionOutput::RowsAffected(n) => assert_eq!(n, 3),
            other => panic!("expected rows affected, got {other:?}"),
        }

        run(&config, &format!("DROP TABLE {table}"), None)
            .await
            .unwrap();
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
        run(
            &config,
            &format!("CREATE TABLE {a} (id int PRIMARY KEY)"),
            None,
        )
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
        run(&config, &format!("DROP TABLE {b}, {a}"), None)
            .await
            .unwrap();

        assert!(schema
            .tables
            .iter()
            .any(|t| t.table_name == b && t.schema_name == "public"));
        assert!(schema.relationships.iter().any(|r| r.source_table_name == b
            && r.source_column_name == "a_id"
            && r.target_table_name == a));
        assert_eq!(cols.len(), 2);
        assert!(cols
            .iter()
            .any(|c| c.name == "id" && c.is_primary_key && !c.is_nullable));
        assert!(cols.iter().any(|c| c.name == "a_id" && c.is_nullable));
    }

    #[tokio::test]
    async fn statement_splitting_handles_dollar_quotes() {
        let driver = PostgresDriver::new();
        let statements = driver.split_statements("SELECT 1; SELECT $$a;b$$;");
        assert_eq!(statements.len(), 2);
    }

    /// Run `text` the way the QueryEngine does: split, then execute each unit.
    async fn run_units(config: &ConnectionConfig, text: &str) -> Vec<ExecutionOutput> {
        let driver = PostgresDriver::new();
        let mut outputs = Vec::new();
        for unit in driver.split_statements(text) {
            outputs.push(run(config, &unit, None).await.unwrap());
        }
        outputs
    }

    fn first_cell(output: &ExecutionOutput) -> CellValue {
        match output {
            ExecutionOutput::ResultSets { sets, .. } => sets[0].rows[0][0].clone(),
            other => panic!("expected result sets, got {other:?}"),
        }
    }

    #[ignore]
    #[tokio::test]
    async fn rollback_script_does_not_commit() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let table = format!(
            "larik_pg_tx_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        run(&config, &format!("CREATE TABLE {table} (id int)"), None)
            .await
            .unwrap();
        run(
            &config,
            &format!("INSERT INTO {table} VALUES (1), (2)"),
            None,
        )
        .await
        .unwrap();

        run_units(&config, &format!("BEGIN; DELETE FROM {table}; ROLLBACK;")).await;
        let count = run(&config, &format!("SELECT count(*)::int FROM {table}"), None)
            .await
            .unwrap();
        run(&config, &format!("DROP TABLE {table}"), None)
            .await
            .unwrap();
        assert_eq!(first_cell(&count), CellValue::Int(2));
    }

    #[ignore]
    #[tokio::test]
    async fn session_state_carries_across_script_statements() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let outputs = run_units(
            &config,
            "CREATE TEMP TABLE t (a int); INSERT INTO t VALUES (7); SET application_name = 'x'; \
             SELECT a, current_setting('application_name') FROM t;",
        )
        .await;
        assert_eq!(outputs.len(), 1);
        match &outputs[0] {
            ExecutionOutput::ResultSets { sets, .. } => {
                assert_eq!(sets.len(), 1);
                assert_eq!(sets[0].rows[0][0], CellValue::Int(7));
                assert_eq!(sets[0].rows[0][1], CellValue::String("x".into()));
            }
            other => panic!("expected result sets, got {other:?}"),
        }
    }

    /// Activity of other backends running `marker`.
    async fn running(config: &ConnectionConfig, marker: &str) -> i64 {
        let sql = format!(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE query LIKE '%{marker}%' AND pid <> pg_backend_pid() AND state = 'active'"
        );
        match first_cell(&run(config, &sql, None).await.unwrap()) {
            CellValue::Int(n) => n,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[ignore]
    #[tokio::test]
    async fn dropping_execute_cancels_server_query() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let marker = format!(
            "larik_cancel_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let sql = format!("SELECT pg_sleep(30), '{marker}'");
        let execution = run(&config, &sql, None);
        // Drop the future mid-query, like the QueryEngine's cancel.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(1500), execution).await;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert_eq!(running(&config, &marker).await, 0);
    }

    #[ignore]
    #[tokio::test]
    async fn truncation_cancels_remaining_rows() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let marker = format!(
            "larik_trunc_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        // Rows trickle out slowly; without a cancel the server keeps going.
        let sql = format!("SELECT g, pg_sleep(0.05), '{marker}' FROM generate_series(1, 2000) g");
        let driver = PostgresDriver::new();
        let output = driver
            .execute(
                &config,
                &sql,
                ExecuteOptions {
                    database: None,
                    row_limit: 2,
                },
            )
            .await
            .unwrap();
        assert!(matches!(&output, ExecutionOutput::ResultSets { sets, .. } if sets[0].truncated));
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert_eq!(running(&config, &marker).await, 0);
    }

    #[ignore]
    #[tokio::test]
    async fn decodes_awkward_types() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let sql = r#"
            SELECT
                '1 year 2 mons 3 days 04:05:06.5'::interval AS iv,
                12345678901234567890.1234567890::numeric AS big,
                'NaN'::numeric AS nan,
                12.34::money AS m,
                '10.0.0.0/8'::cidr AS c,
                '10.1.2.3/16'::inet AS i,
                ARRAY['a', NULL, 'NULL'] AS t_nulls,
                ARRAY[[1, 2], [3, NULL]] AS md,
                '10:11:12+02'::timetz AS tz,
                B'10101'::bit(5) AS b,
                'infinity'::timestamp AS inf,
                ARRAY['2024-01-02'::date] AS dates,
                'r'::"char" AS ch,
                int4range(1, 5) AS rng
        "#;
        match run(&config, sql, None).await.unwrap() {
            ExecutionOutput::ResultSets { sets, .. } => {
                let r = &sets[0].rows[0];
                assert_eq!(
                    r[0],
                    CellValue::String("1 year 2 mons 3 days 04:05:06.5".into())
                );
                assert_eq!(
                    r[1],
                    CellValue::String("12345678901234567890.1234567890".into())
                );
                assert_eq!(r[2], CellValue::String("NaN".into()));
                assert_eq!(r[3], CellValue::Float(12.34));
                assert_eq!(r[4], CellValue::String("10.0.0.0/8".into()));
                assert_eq!(r[5], CellValue::String("10.1.2.3/16".into()));
                assert_eq!(r[6], CellValue::String("{a,NULL,\"NULL\"}".into()));
                assert_eq!(r[7], CellValue::String("{{1,2},{3,NULL}}".into()));
                assert_eq!(r[8], CellValue::DateTime("10:11:12+02".into()));
                assert_eq!(r[9], CellValue::String("10101".into()));
                assert_eq!(r[10], CellValue::DateTime("infinity".into()));
                assert_eq!(r[11], CellValue::String("{2024-01-02}".into()));
                assert_eq!(r[12], CellValue::String("r".into()));
                // Range binary format isn't text: shown as bytes, not garbage text.
                assert!(matches!(r[13], CellValue::Binary(_)));
            }
            other => panic!("expected result sets, got {other:?}"),
        }
    }

    #[ignore]
    #[tokio::test]
    async fn routines_include_zero_arg_and_overloads() {
        if !url_available() {
            return;
        }
        let config = live_config().unwrap();
        let schema = format!(
            "larik_fn_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        run(&config, &format!("CREATE SCHEMA {schema}"), None)
            .await
            .unwrap();
        for ddl in [
            format!("CREATE FUNCTION {schema}.noargs() RETURNS int LANGUAGE sql AS 'SELECT 1'"),
            format!("CREATE FUNCTION {schema}.ov(a int) RETURNS int LANGUAGE sql AS 'SELECT a'"),
            format!(
                "CREATE FUNCTION {schema}.ov(a int, b text) RETURNS int LANGUAGE sql AS 'SELECT a'"
            ),
            format!(
                "CREATE FUNCTION {schema}.outs(OUT x int, IN y int) LANGUAGE sql AS 'SELECT y'"
            ),
        ] {
            run(&config, &ddl, None).await.unwrap();
        }

        let driver = PostgresDriver::new();
        let info = driver
            .fetch_schema(&config, &config.database, Some(&schema))
            .await;
        run(&config, &format!("DROP SCHEMA {schema} CASCADE"), None)
            .await
            .unwrap();
        let routines = info.unwrap().routines;

        let named = |name: &str| {
            routines
                .iter()
                .filter(|r| r.routine_name == name)
                .collect::<Vec<_>>()
        };
        assert_eq!(named("noargs").len(), 1);
        assert!(named("noargs")[0].parameters.is_empty());

        let mut overload_sizes: Vec<usize> =
            named("ov").iter().map(|r| r.parameters.len()).collect();
        overload_sizes.sort();
        assert_eq!(overload_sizes, vec![1, 2]);

        let outs = &named("outs")[0].parameters;
        assert_eq!(outs.len(), 2);
        assert_eq!(
            (outs[0].name.as_str(), outs[0].parameter_mode.as_str()),
            ("x", "OUT")
        );
        assert_eq!(
            (outs[1].name.as_str(), outs[1].parameter_mode.as_str()),
            ("y", "IN")
        );
        assert_eq!(outs[1].data_type, "integer");
    }
}
