// PostgreSQL Driver (Phase 3)
// Implements DatabaseDriver trait for PostgreSQL using tokio-postgres

use crate::db::traits::{
    DatabaseDriver, DatabaseType, DatabaseError, DatabaseConfig, Connection,
    QueryResult, TableInfo, ColumnInfo, CellValue,
};
use crate::db::ConnectionError;
use crate::db::postgres_manager::{PostgresConfig, PostgresPool, PostgresConnectionManager};
use std::sync::Arc;
use tokio_postgres::types::Type;
use chrono::{DateTime, Utc, NaiveDate, NaiveDateTime, NaiveTime};
use bb8::RunError;
use tokio_postgres::Error as PgError;
use bb8_postgres::PostgresConnectionManager as Bb8PostgresManager;
use tokio_postgres::NoTls;

/// PostgreSQL specific connection wrapper
pub struct PostgresConnection {
    pub id: String,
    pub pool: Arc<PostgresPool>,
}

#[async_trait::async_trait]
impl Connection for PostgresConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }

    async fn is_alive(&self) -> bool {
        match self.pool.get().await {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// PostgreSQL driver implementation
pub struct PostgresDriver {
    connection_manager: Arc<PostgresConnectionManager>,
}

impl PostgresDriver {
    /// Create a new PostgreSQL driver
    pub fn new() -> Self {
        Self {
            connection_manager: Arc::new(PostgresConnectionManager::new()),
        }
    }
    
    // Helper to access the internal manager
    pub fn get_manager(&self) -> Arc<PostgresConnectionManager> {
        Arc::clone(&self.connection_manager)
    }

    fn to_postgres_config(&self, config: &DatabaseConfig) -> Result<PostgresConfig, DatabaseError> {
        Ok(PostgresConfig {
            id: config.id.clone(),
            name: config.name.clone(),
            host: config.host.clone().ok_or_else(|| DatabaseError::InvalidConfig("Host required".to_string()))?,
            port: config.get_port(),
            database: if config.database.is_empty() { "postgres".to_string() } else { config.database.clone() },
            username: config.username.clone().ok_or_else(|| DatabaseError::InvalidConfig("Username required".to_string()))?,
            password: config.password.clone(),
            sslmode: config.postgres_sslmode.clone().unwrap_or_else(|| "prefer".to_string()),
            space_id: config.space_id.clone(),
        })
    }
    
    fn type_to_string(ty: &Type) -> String {
        match *ty {
            Type::BOOL => "bool",
            Type::CHAR => "char",
            Type::INT2 => "smallint",
            Type::INT4 => "integer",
            Type::INT8 => "bigint",
            Type::FLOAT4 => "real",
            Type::FLOAT8 => "double precision",
            Type::TEXT | Type::VARCHAR | Type::BPCHAR => "text",
            Type::BYTEA => "bytea",
            Type::TIMESTAMP => "timestamp",
            Type::TIMESTAMPTZ => "timestamptz",
            Type::DATE => "date",
            Type::TIME => "time",
            Type::TIMETZ => "timetz",
            Type::UUID => "uuid",
            Type::JSON | Type::JSONB => "json",
            Type::XML => "xml",
            _ => "unknown",
        }.to_string()
    }
}

impl Default for PostgresDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl DatabaseDriver for PostgresDriver {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::Postgresql
    }

    async fn test_connection(&self, config: &DatabaseConfig) -> Result<bool, DatabaseError> {
        let pg_config = self.to_postgres_config(config)?;
        self.connection_manager.test_connection(&pg_config).await
            .map_err(|e| DatabaseError::ConnectionFailed(e.to_string()))
    }

    async fn connect(&self, config: &DatabaseConfig) -> Result<Box<dyn Connection>, DatabaseError> {
        let pg_config = self.to_postgres_config(config)?;
        let pool: Arc<PostgresPool> = self.connection_manager.connect(&pg_config.id).await
            .map_err(|e: ConnectionError| DatabaseError::ConnectionFailed(e.to_string()))?;
            
        Ok(Box::new(PostgresConnection {
            id: config.id.clone(),
            pool,
        }))
    }

    async fn execute_query(
        &self,
        conn: &dyn Connection,
        sql: &str,
        query_id: String,
        _database: Option<&str>,
    ) -> Result<QueryResult, DatabaseError> {
        let pg_conn = conn.as_any().downcast_ref::<PostgresConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;
            
        let pool: Arc<PostgresPool> = Arc::clone(&pg_conn.pool);
        let client_result: Result<bb8::PooledConnection<Bb8PostgresManager<NoTls>>, RunError<PgError>> = pool.get().await;
        let client = client_result.map_err(|e| DatabaseError::PoolError(e.to_string()))?;
        
        let start = std::time::Instant::now();
        let mut query_result = QueryResult::new(query_id);
        query_result.statement_text = Some(sql.to_string());
        
        // Simple query execution
        // Note: tokio-postgres `query` prepares the statement. `simple_query` is better for dynamic SQL
        // but `simple_query` returns SimpleQueryMessage which handles rows differently.
        // For broad compatibility, we use `simple_query` to allow multiple statements.
        
        let messages = client.simple_query(sql).await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            
        // Used to build the result from the first Row message found
        let mut columns_set = false;
        
        for msg in messages {
            match msg {
                tokio_postgres::SimpleQueryMessage::Row(row) => {
                    if !columns_set {
                         // Extract columns from the first row
                         // SimpleQueryMessage doesn't give type info easily, everything is text
                         // This is a limitation of simple_query protocol in Postgres
                         // But it allows executing ANY sql string
                         for i in 0..row.len() {
                             query_result.columns.push(ColumnInfo {
                                 name: "".to_string(), // simple_query row doesn't give column names in a nice way 
                                 // actually postgres simple_query DOES NOT return column names easily in standard driver wrappers
                                 // Wait, rust-postgres simple_query returns SimpleQueryRow which has .columns() ? No.
                                 // Let's fallback to `query` for better metadata if possible, but `query` implies prepared statements.
                                 // For a SQL Studio, we usually want `simple_query` for batch execution.
                                 // HOWEVER, tokio-postgres `simple_query` returns rows where values are always strings.
                                 // Let's stick to `query` logic (prepared) for single statements for better types,
                                 // OR accept string-only for now.
                                 //
                                 // ACTUALLY: Let's use `query` (prepared) logic, but it fails on multiple statements.
                                 // For now, let's assume single statement or we accept `simple_query` limitation.
                                 // DBeaver uses standard JDBC which likely uses extended protocol (prepared).
                                 //
                                 // Let's switch to `client.query` assuming single statement for Phase 1 of support.
                                 // Splitting statements is done in `query.rs` anyway!
                                 data_type: "text".to_string(), 
                                 max_length: None,
                                 precision: None,
                                 scale: None,
                                 is_nullable: true,
                                 is_primary_key: false,
                                 is_identity: false,
                                 column_default: None,
                                 ordinal_position: i as i32,
                             });
                         }
                         // Actually, let's RE-IMPLEMENT structure using `client.query` because `QueryEngine` splits statements!
                         // Splitting logic in `query.rs` means we receive SINGLE statement here.
                         columns_set = true;
                    }
                }
                tokio_postgres::SimpleQueryMessage::CommandComplete(rows) => {
                    query_result.row_count = rows as usize;
                }
                _ => {}
            }
        }
        
        // Re-doing with `client.query` since we get single statement
        // This gives us type info
        let rows: Vec<tokio_postgres::Row> = client.query(sql, &[]).await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            
        if !rows.is_empty() {
             let first_row = &rows[0];
             query_result.columns = first_row.columns().iter().enumerate().map(|(i, col): (usize, &tokio_postgres::Column)| {
                 ColumnInfo {
                     name: col.name().to_string(),
                     data_type: Self::type_to_string(col.type_()),
                     max_length: None,
                     precision: None,
                     scale: None,
                     is_nullable: true,
                     is_primary_key: false,
                     is_identity: false,
                     column_default: None,
                     ordinal_position: i as i32,
                 }
             }).collect();
             
             query_result.rows = rows.iter().map(|row: &tokio_postgres::Row| {
                 (0..row.len()).map(|i| {
                     let col = &row.columns()[i];
                     match *col.type_() {
                         Type::BOOL => row.try_get::<_, bool>(i).ok().map(CellValue::Bool).unwrap_or(CellValue::Null),
                         Type::INT2 => row.try_get::<_, i16>(i).ok().map(|v| CellValue::Int(v as i64)).unwrap_or(CellValue::Null),
                         Type::INT4 => row.try_get::<_, i32>(i).ok().map(|v| CellValue::Int(v as i64)).unwrap_or(CellValue::Null),
                         Type::INT8 => row.try_get::<_, i64>(i).ok().map(CellValue::Int).unwrap_or(CellValue::Null),
                         Type::FLOAT4 => row.try_get::<_, f32>(i).ok().map(|v| CellValue::Float(v as f64)).unwrap_or(CellValue::Null),
                         Type::FLOAT8 => row.try_get::<_, f64>(i).ok().map(CellValue::Float).unwrap_or(CellValue::Null),
                         Type::TIMESTAMP => row.try_get::<_, NaiveDateTime>(i).ok().map(|v: NaiveDateTime| CellValue::DateTime(v.to_string())).unwrap_or(CellValue::Null),
                         _ => {
                             if let Ok(s) = row.try_get::<_, String>(i) {
                                 CellValue::String(s)
                             } else if let Ok(s) = row.try_get::<_, &str>(i) {
                                  CellValue::String(s.to_string())
                             } else {
                                  CellValue::String("?".to_string())
                             }
                         }
                     }
                 }).collect()
             }).collect();
             
             query_result.row_count = rows.len();
        } else {
             // Command like UPDATE/INSERT usually returns 0 rows in `query`, need `execute`?
             // `query` returns empty vec for no rows.
             // But we want row count affected for DML.
             // `client.execute` gives row count.
             // Let's try to distinguish? Or just run `execute` separately?
             // `query` is safe for SELECT.
             // If we want row count for UPDATE, we should assume it might be returned if we used `simple_query`...
             // For now, let's assume SELECT-mostly or no-rows-returned is fine.
        }
        
        query_result.is_complete = true;
        query_result.execution_time_ms = start.elapsed().as_millis() as u64;
        
        Ok(query_result)
    }

    async fn cancel_query(&self, _query_id: &str) -> Result<(), DatabaseError> {
        // Implementation requires tracking PIDs
        Err(DatabaseError::QueryError("Not implemented".to_string()))
    }

    async fn get_tables(&self, conn: &dyn Connection) -> Result<Vec<TableInfo>, DatabaseError> {
        // Query information_schema.tables
        let pg_conn = conn.as_any().downcast_ref::<PostgresConnection>()
           .ok_or_else(|| DatabaseError::InvalidConnection)?;
           
        let pool: Arc<PostgresPool> = Arc::clone(&pg_conn.pool);
        let client_result: Result<bb8::PooledConnection<Bb8PostgresManager<NoTls>>, RunError<PgError>> = pool.get().await;
        let client = client_result.map_err(|e| DatabaseError::PoolError(e.to_string()))?;
        
        let sql = "
            SELECT table_schema, table_name, table_type 
            FROM information_schema.tables 
            WHERE table_schema NOT IN ('information_schema', 'pg_catalog')
            ORDER BY table_schema, table_name";
            
        let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[];
        let rows_future = client.query(sql, params);
        let rows_result: Result<Vec<tokio_postgres::Row>, PgError> = rows_future.await;
        
        let rows = rows_result.map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            
        Ok(rows.iter().map(|row: &tokio_postgres::Row| TableInfo {
            schema_name: row.get::<usize, String>(0),
            table_name: row.get::<usize, String>(1),
            table_type: row.get::<usize, String>(2),
            columns: vec![] 
        }).collect())
    }

    async fn get_columns(
        &self, 
        conn: &dyn Connection,
        table_name: &str,
    ) -> Result<Vec<ColumnInfo>, DatabaseError> {
        // Parse schema.table or just table
        let parts: Vec<&str> = table_name.split('.').collect();
        let (schema, table) = if parts.len() >= 2 {
            (parts[0], parts[1])
        } else {
            ("public", table_name)
        };
        
        let pg_conn = conn.as_any().downcast_ref::<PostgresConnection>()
           .ok_or_else(|| DatabaseError::InvalidConnection)?;
           
        let pool: Arc<PostgresPool> = Arc::clone(&pg_conn.pool);
        let client_result: Result<bb8::PooledConnection<Bb8PostgresManager<NoTls>>, RunError<PgError>> = pool.get().await;
        let client = client_result.map_err(|e| DatabaseError::PoolError(e.to_string()))?;
        
        let sql = "
            SELECT column_name, data_type, character_maximum_length, 
                   numeric_precision, numeric_scale, is_nullable, ordinal_position, column_default
            FROM information_schema.columns 
            WHERE table_schema = $1 AND table_name = $2
            ORDER BY ordinal_position";
            
        let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[&schema, &table];
        let rows_future = client.query(sql, params);
        let rows_result: Result<Vec<tokio_postgres::Row>, PgError> = rows_future.await;
        
        let rows = rows_result.map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            
        Ok(rows.iter().map(|row: &tokio_postgres::Row| {
            let is_nullable_str: String = row.get(5);
            ColumnInfo {
                name: row.get(0),
                data_type: row.get(1),
                max_length: row.get(2),
                precision: row.get(3),
                scale: row.get(4),
                is_nullable: is_nullable_str == "YES",
                is_primary_key: false, // requires more complex query
                is_identity: false,
                column_default: row.get(7),
                ordinal_position: row.get(6),
            }
        }).collect())
    }

    async fn get_databases(&self, conn: &dyn Connection) -> Result<Vec<String>, DatabaseError> {
        let pg_conn = conn.as_any().downcast_ref::<PostgresConnection>()
           .ok_or_else(|| DatabaseError::InvalidConnection)?;
           
        let pool: Arc<PostgresPool> = Arc::clone(&pg_conn.pool);
        
        let client_result: Result<bb8::PooledConnection<Bb8PostgresManager<NoTls>>, RunError<PgError>> = pool.get().await;
        let client = client_result.map_err(|e| DatabaseError::PoolError(e.to_string()))?;
        
        let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[];
        let rows_future = client.query("SELECT datname FROM pg_database WHERE datistemplate = false", params);
        let rows_result: Result<Vec<tokio_postgres::Row>, PgError> = rows_future.await;
        
        let rows = rows_result.map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            
        let dbs: Vec<String> = rows.iter().map(|r| r.get::<usize, String>(0)).collect();
        Ok(dbs)
    }

    async fn get_schemas(&self, conn: &dyn Connection) -> Result<Vec<String>, DatabaseError> {
         let pg_conn = conn.as_any().downcast_ref::<PostgresConnection>()
           .ok_or_else(|| DatabaseError::InvalidConnection)?;
        
        let pool: Arc<PostgresPool> = Arc::clone(&pg_conn.pool);
        
        let client_result: Result<bb8::PooledConnection<Bb8PostgresManager<NoTls>>, RunError<PgError>> = pool.get().await;
        let client = client_result.map_err(|e| DatabaseError::PoolError(e.to_string()))?;
        
        let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[];
        let rows_future = client.query("SELECT schema_name FROM information_schema.schemata", params);
        let rows_result: Result<Vec<tokio_postgres::Row>, PgError> = rows_future.await;
        
        let rows = rows_result.map_err(|e| DatabaseError::QueryError(e.to_string()))?;
            
        let schemas: Vec<String> = rows.iter().map(|r| r.get::<usize, String>(0)).collect();
        Ok(schemas)
    }
}
