// MongoDB driver
// Queries are written in mongo-shell syntax (see shell.rs), parsed without a
// JavaScript engine and mapped onto the official Rust driver.

mod connection;
mod convert;
mod exec;
mod schema;
pub mod shell;
pub mod sql;

use async_trait::async_trait;

use crate::db::config::ConnectionConfig;
use crate::db::driver::{
    DatabaseDriver, DatabaseEntry, DriverKind, ExecuteOptions, ExecutionOutput,
};
use crate::db::error::ConnectionError;
use crate::db::schema::{ColumnInfo as SchemaColumnInfo, SchemaInfo};
use connection::MongoClients;

/// MongoDB implementation of `DatabaseDriver`
#[derive(Default)]
pub struct MongoDriver {
    clients: MongoClients,
}

impl MongoDriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve the database a statement runs in: the tab's selection, else the
    /// connection's database, else the connection string's default, else the
    /// first accessible non-system database, else `test`.
    async fn target_database(
        config: &ConnectionConfig,
        requested: Option<&str>,
        client: Option<&mongodb::Client>,
    ) -> String {
        if let Some(db) = requested.map(str::trim).filter(|d| !d.is_empty()) {
            return db.to_string();
        }
        let options = if connection::is_connection_string(&config.host) {
            connection::client_options(config).await.ok()
        } else {
            None
        };
        if let Some(db) = connection::configured_database(config, options.as_ref()) {
            return db;
        }
        if let Some(client) = client {
            if let Ok(mut names) = client
                .list_database_names()
                .authorized_databases(true)
                .await
            {
                names.sort();
                if let Some(first) = names
                    .into_iter()
                    .find(|n| !matches!(n.as_str(), "admin" | "config" | "local"))
                {
                    return first;
                }
            }
        }
        "test".to_string()
    }
}

#[async_trait]
impl DatabaseDriver for MongoDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Mongodb
    }

    async fn test_connection(&self, config: &ConnectionConfig) -> Result<(), ConnectionError> {
        let client = connection::connect_client(config).await?;
        client.shutdown().await;
        Ok(())
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
            Some(client) => connection::ping(&client).await.is_ok(),
            None => false,
        }
    }

    async fn list_databases(
        &self,
        config: &ConnectionConfig,
    ) -> Result<Vec<DatabaseEntry>, ConnectionError> {
        let client = self.clients.get_or_create(config).await?;
        match client
            .list_database_names()
            .authorized_databases(true)
            .await
        {
            Ok(mut names) => {
                names.sort();
                Ok(names
                    .into_iter()
                    .map(|name| DatabaseEntry {
                        name,
                        has_access: true,
                    })
                    .collect())
            }
            // Users without listDatabases can still work in their own database
            Err(_) => Ok(vec![DatabaseEntry {
                name: Self::target_database(config, None, None).await,
                has_access: true,
            }]),
        }
    }

    fn split_statements(&self, text: &str) -> Vec<String> {
        shell::split_statements(text)
    }

    /// Note on cancellation: dropping this future stops fetching and kills any
    /// open cursor, but an operation already running on the server (e.g. a long
    /// aggregation's first batch) finishes there.
    async fn execute(
        &self,
        config: &ConnectionConfig,
        statement: &str,
        options: ExecuteOptions<'_>,
    ) -> Result<ExecutionOutput, ConnectionError> {
        // SELECT statements are compiled to an aggregation pipeline (read-only)
        if shell::is_sql_statement(statement) {
            let compiled = sql::compile(statement).map_err(ConnectionError::QueryError)?;
            let client = self.clients.get_or_create(config).await?;
            let database = Self::target_database(config, options.database, Some(&client)).await;
            return exec::execute_sql(&client, &database, compiled, options.row_limit).await;
        }

        let parsed = shell::parse_single(statement)
            .map_err(|e| ConnectionError::QueryError(e.describe(statement)))?;
        let client = self.clients.get_or_create(config).await?;
        let database = Self::target_database(config, options.database, Some(&client)).await;
        exec::execute(&client, &database, parsed, options.row_limit).await
    }

    async fn fetch_schema(
        &self,
        config: &ConnectionConfig,
        database: &str,
        _schema_filter: Option<&str>,
    ) -> Result<SchemaInfo, ConnectionError> {
        let client = self.clients.get_or_create(config).await?;
        schema::fetch_schema(&client, database).await
    }

    async fn fetch_table_columns(
        &self,
        config: &ConnectionConfig,
        database: &str,
        _schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<SchemaColumnInfo>, ConnectionError> {
        let client = self.clients.get_or_create(config).await?;
        schema::sample_columns(&client.database(database), table_name).await
    }
}

/// End-to-end tests against a real server. Run with:
///   LARIK_TEST_MONGODB_URL=mongodb://localhost:27017 cargo test -- --ignored mongodb
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod live_tests {
    use super::*;
    use crate::db::query::CellValue;

    fn live_config() -> Option<ConnectionConfig> {
        let url = std::env::var("LARIK_TEST_MONGODB_URL").ok()?;
        let mut config = ConnectionConfig::new(
            DriverKind::Mongodb,
            "live".to_string(),
            url,
            27017,
            String::new(),
            String::new(),
            String::new(),
        );
        config.id = format!("live-{}", uuid::Uuid::new_v4());
        Some(config)
    }

    async fn run(
        driver: &MongoDriver,
        config: &ConnectionConfig,
        db: &str,
        statement: &str,
    ) -> Result<ExecutionOutput, ConnectionError> {
        driver
            .execute(
                config,
                statement,
                ExecuteOptions {
                    database: Some(db),
                    row_limit: 3,
                    read_only: false,
                },
            )
            .await
    }

    fn sets(output: ExecutionOutput) -> Vec<crate::db::driver::ResultSet> {
        match output {
            ExecutionOutput::ResultSets { sets, .. } => sets,
            other => panic!("expected result sets, got {other:?}"),
        }
    }

    fn affected(output: ExecutionOutput) -> usize {
        match output {
            ExecutionOutput::RowsAffected(n) => n,
            other => panic!("expected rows affected, got {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore = "needs LARIK_TEST_MONGODB_URL"]
    async fn mongodb_end_to_end() {
        let Some(config) = live_config() else {
            eprintln!("LARIK_TEST_MONGODB_URL not set; skipping");
            return;
        };
        let driver = MongoDriver::new();
        let db = format!("larik_test_{}", &config.id[5..13]);

        driver.test_connection(&config).await.unwrap();
        driver.connect(&config).await.unwrap();
        assert!(driver.is_healthy(&config.id).await);

        // Writes
        let inserted = run(&driver, &config, &db, r#"db.users.insertMany([
            { name: "Ada", age: 36, tags: ["math"], address: { city: "London" }, joined: ISODate("2020-01-02") },
            { name: "Grace", age: 45, tags: ["navy", "cobol"], address: { city: "New York" } },
            { name: "Linus", age: 28 },
            { name: "Margaret", age: 33, address: { city: "Boston" } },
            { name: "Barbara", age: 52 }
        ])"#).await.unwrap();
        assert_eq!(affected(inserted), 5);
        let one = run(
            &driver,
            &config,
            &db,
            r#"db.getCollection("order-items").insertOne({ sku: "X1", qty: NumberLong(2) })"#,
        )
        .await
        .unwrap();
        assert_eq!(affected(one), 1);

        // find with filter, projection, sort, limit
        let found = sets(run(&driver, &config, &db, "db.users.find({ age: { $gte: 30 } }, { _id: 0, name: 1, age: 1 }).sort({ age: -1 }).limit(2)").await.unwrap());
        let names: Vec<&str> = found[0].columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["name", "age"]);
        assert_eq!(found[0].rows.len(), 2);
        assert!(matches!(&found[0].rows[0][0], CellValue::String(s) if s == "Barbara"));
        assert!(!found[0].truncated);

        // Row limit (3) truncates an unbounded find
        let all = sets(run(&driver, &config, &db, "db.users.find()").await.unwrap());
        assert_eq!(all[0].rows.len(), 3);
        assert!(all[0].truncated);

        // Regex, nested field and ISODate filters
        let regex = sets(
            run(
                &driver,
                &config,
                &db,
                r#"db.users.find({ name: /^g/i, "address.city": "New York" })"#,
            )
            .await
            .unwrap(),
        );
        assert_eq!(regex[0].rows.len(), 1);
        let dated = sets(
            run(
                &driver,
                &config,
                &db,
                r#"db.users.countDocuments({ joined: { $lt: new Date("2021-01-01") } })"#,
            )
            .await
            .unwrap(),
        );
        assert!(matches!(dated[0].rows[0][0], CellValue::Int(1)));

        // aggregate
        let agg = sets(
            run(
                &driver,
                &config,
                &db,
                r#"db.users.aggregate([
            { $match: { age: { $gt: 30 } } },
            { $group: { _id: null, avgAge: { $avg: "$age" }, n: { $sum: 1 } } }
        ])"#,
            )
            .await
            .unwrap(),
        );
        assert!(matches!(agg[0].rows[0][2], CellValue::Int(4)));

        // distinct, count, show, runCommand
        let distinct = sets(
            run(
                &driver,
                &config,
                &db,
                r#"db.users.distinct("address.city")"#,
            )
            .await
            .unwrap(),
        );
        assert_eq!(distinct[0].rows.len(), 3);
        let count = sets(
            run(
                &driver,
                &config,
                &db,
                "db.users.find({ age: { $lt: 40 } }).count()",
            )
            .await
            .unwrap(),
        );
        assert!(matches!(count[0].rows[0][0], CellValue::Int(3)));
        let collections = sets(
            run(&driver, &config, &db, "show collections")
                .await
                .unwrap(),
        );
        assert_eq!(collections[0].rows.len(), 2);
        let ping = sets(
            run(&driver, &config, &db, "db.runCommand({ ping: 1 })")
                .await
                .unwrap(),
        );
        assert!(ping[0].columns.iter().any(|c| c.name == "ok"));

        // SQL SELECT compiled to aggregation pipelines
        affected(run(&driver, &config, &db, r#"db.orders.insertMany([
            { user: "Ada", total: 10 }, { user: "Ada", total: 5 }, { user: "Grace", total: 7 }, { user: "Nobody", total: 1 }
        ])"#).await.unwrap());
        let sql_sets = |output: ExecutionOutput| match output {
            ExecutionOutput::ResultSets {
                sets,
                generated_query,
                ..
            } => {
                assert!(
                    generated_query
                        .as_deref()
                        .is_some_and(|g| g.contains(".aggregate([")),
                    "{generated_query:?}"
                );
                sets
            }
            other => panic!("expected result sets, got {other:?}"),
        };
        let cell_strings = |set: &crate::db::driver::ResultSet, col: usize| -> Vec<String> {
            set.rows
                .iter()
                .map(|r| match &r[col] {
                    CellValue::String(s) => s.clone(),
                    other => format!("{other:?}"),
                })
                .collect()
        };

        let top = sql_sets(
            run(
                &driver,
                &config,
                &db,
                "SELECT name, age FROM users WHERE age > 30 ORDER BY age DESC LIMIT 2",
            )
            .await
            .unwrap(),
        );
        assert_eq!(cell_strings(&top[0], 0), vec!["Barbara", "Grace"]);

        let by_city = sql_sets(run(&driver, &config, &db, "SELECT address.city AS city, COUNT(*) AS n FROM users WHERE address.city IS NOT NULL GROUP BY address.city ORDER BY city").await.unwrap());
        assert_eq!(
            cell_strings(&by_city[0], 0),
            vec!["Boston", "London", "New York"]
        );
        assert!(by_city[0]
            .rows
            .iter()
            .all(|r| matches!(r[1], CellValue::Int(1))));

        let totals = sql_sets(run(&driver, &config, &db, "SELECT COUNT(*) AS total, COUNT(address) AS with_address, MAX(age) oldest FROM users").await.unwrap());
        assert!(
            matches!(
                totals[0].rows[0][..],
                [CellValue::Int(5), CellValue::Int(3), CellValue::Int(52)]
            ),
            "{:?}",
            totals[0].rows
        );

        let like = sql_sets(
            run(
                &driver,
                &config,
                &db,
                "SELECT name FROM users WHERE name LIKE 'G%' OR name ILIKE 'lin%'",
            )
            .await
            .unwrap(),
        );
        assert_eq!(cell_strings(&like[0], 0), vec!["Grace", "Linus"]);

        let joined = sql_sets(run(&driver, &config, &db, "SELECT u.name, SUM(o.total) AS spent FROM orders o JOIN users u ON o.user = u.name GROUP BY u.name ORDER BY spent DESC").await.unwrap());
        assert_eq!(cell_strings(&joined[0], 0), vec!["Ada", "Grace"]);
        assert!(matches!(joined[0].rows[0][1], CellValue::Int(15)));

        let left = sql_sets(run(&driver, &config, &db, "SELECT o.user, u.age FROM orders o LEFT JOIN users u ON o.user = u.name WHERE o.total < 2").await.unwrap());
        assert_eq!(cell_strings(&left[0], 0), vec!["Nobody"]);
        assert!(matches!(left[0].rows[0][1], CellValue::Null));

        let computed = sql_sets(run(&driver, &config, &db, "SELECT UPPER(name) AS n, CASE WHEN age >= 40 THEN 'senior' ELSE 'junior' END band FROM users WHERE tags IS NOT NULL ORDER BY n").await.unwrap());
        assert_eq!(cell_strings(&computed[0], 0), vec!["ADA", "GRACE"]);
        assert_eq!(cell_strings(&computed[0], 1), vec!["junior", "senior"]);

        let distinct = sql_sets(
            run(
                &driver,
                &config,
                &db,
                "SELECT DISTINCT user FROM orders ORDER BY user",
            )
            .await
            .unwrap(),
        );
        assert_eq!(
            cell_strings(&distinct[0], 0),
            vec!["Ada", "Grace", "Nobody"]
        );

        // Row limit (3) still applies to SQL results
        let limited = sql_sets(
            run(&driver, &config, &db, "SELECT * FROM users")
                .await
                .unwrap(),
        );
        assert!(limited[0].truncated);

        let sql_error = run(&driver, &config, &db, "SELECT * FROM users WHERE")
            .await
            .unwrap_err();
        assert!(
            matches!(sql_error, ConnectionError::QueryError(ref m) if m.starts_with("SQL error")),
            "{sql_error:?}"
        );
        let write = run(&driver, &config, &db, "select 1 from users; ").await;
        assert!(write.is_ok());

        // update / upsert / delete
        let updated = run(
            &driver,
            &config,
            &db,
            r#"db.users.updateMany({ age: { $lt: 35 } }, { $inc: { age: 1 } })"#,
        )
        .await
        .unwrap();
        assert_eq!(affected(updated), 2);
        let upserted = run(
            &driver,
            &config,
            &db,
            r#"db.users.updateOne({ name: "Edsger" }, { $set: { age: 72 } }, { upsert: true })"#,
        )
        .await
        .unwrap();
        assert_eq!(affected(upserted), 1);
        let deleted = run(
            &driver,
            &config,
            &db,
            r#"db.users.deleteMany({ name: { $in: ["Edsger", "Linus"] } })"#,
        )
        .await
        .unwrap();
        assert_eq!(affected(deleted), 2);

        // Errors: syntax is a QueryError with a position, server errors are QueryErrors too
        let syntax = run(&driver, &config, &db, "db.users.find({ age: })")
            .await
            .unwrap_err();
        assert!(
            matches!(syntax, ConnectionError::QueryError(ref m) if m.contains("line 1")),
            "{syntax:?}"
        );
        let server = run(
            &driver,
            &config,
            &db,
            r#"db.users.find({ age: { $bogus: 1 } })"#,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(server, ConnectionError::QueryError(_)),
            "{server:?}"
        );
        let unsafe_delete = run(&driver, &config, &db, "db.users.deleteMany()")
            .await
            .unwrap_err();
        assert!(matches!(unsafe_delete, ConnectionError::QueryError(_)));

        // Schema inference and database listing
        let schema = driver.fetch_schema(&config, &db, None).await.unwrap();
        let users = schema
            .tables
            .iter()
            .find(|t| t.table_name == "users")
            .unwrap();
        assert_eq!(users.table_type, "COLLECTION");
        let fields: Vec<&str> = users.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(
            fields.contains(&"_id") && fields.contains(&"address.city") && fields.contains(&"tags"),
            "{fields:?}"
        );
        assert!(schema.tables.iter().any(|t| t.table_name == "order-items"));
        let databases = driver.list_databases(&config).await.unwrap();
        assert!(databases.iter().any(|d| d.name == db));

        // Clean up
        sets(
            run(&driver, &config, &db, "db.runCommand({ dropDatabase: 1 })")
                .await
                .unwrap(),
        );
        driver.disconnect(&config.id).await;
        assert!(!driver.is_connected(&config.id).await);
    }
}
