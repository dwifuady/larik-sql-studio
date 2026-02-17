// MS-SQL Driver (Phase 2 - Refactor Existing Code)
// Implements DatabaseDriver trait for MS-SQL using tiberius

use crate::db::traits::{
    DatabaseDriver, DatabaseType, DatabaseError, DatabaseConfig, Connection,
    QueryResult, TableInfo, ColumnInfo, CellValue,
};
use crate::db::connection::{ConnectionConfig as MssqlConnectionConfig, MssqlPool};
use std::sync::Arc;
use tiberius::{Row, ColumnType};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

// Re-export types from the old connection module for backward compatibility
pub use crate::db::connection::{
    MssqlConnectionManager, ConnectionInfo, ConnectionConfigUpdate,
};

/// MS-SQL specific connection wrapper
pub struct MssqlConnection {
    pub id: String,
    pub pool: Arc<MssqlPool>,
}

#[async_trait::async_trait]
impl Connection for MssqlConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }

    async fn is_alive(&self) -> bool {
        match self.pool.get().await {
            Ok(_conn) => true,
            Err(_) => false,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// MS-SQL driver implementation
pub struct MssqlDriver {
    connection_manager: Arc<MssqlConnectionManager>,
}

impl MssqlDriver {
    /// Create a new MS-SQL driver
    pub fn new() -> Self {
        Self {
            connection_manager: Arc::new(MssqlConnectionManager::new()),
        }
    }

    /// Convert unified DatabaseConfig to MS-SQL specific config
    fn to_mssql_config(&self, config: &DatabaseConfig) -> Result<MssqlConnectionConfig, DatabaseError> {
        let mut mssql_config = MssqlConnectionConfig::new(
            config.name.clone(),
            config.host.clone().ok_or_else(|| DatabaseError::InvalidConfig("Host required for MS-SQL".to_string()))?,
            config.get_port(),
            config.database.clone(),
            config.username.clone().ok_or_else(|| DatabaseError::InvalidConfig("Username required for MS-SQL".to_string()))?,
            config.password.clone(),
        );

        mssql_config.id = config.id.clone();
        mssql_config.space_id = config.space_id.clone();
        mssql_config.encrypt = config.mssql_encrypt.unwrap_or(false);
        mssql_config.trust_certificate = config.mssql_trust_cert.unwrap_or(true);

        Ok(mssql_config)
    }

    /// Convert MS-SQL specific config to unified config
    #[allow(dead_code)]
    fn from_mssql_config(&self, mssql_config: &MssqlConnectionConfig) -> DatabaseConfig {
        DatabaseConfig {
            id: mssql_config.id.clone(),
            name: mssql_config.name.clone(),
            database_type: DatabaseType::Mssql,
            space_id: mssql_config.space_id.clone(),
            host: Some(mssql_config.host.clone()),
            port: Some(mssql_config.port),
            database: mssql_config.database.clone(),
            username: Some(mssql_config.username.clone()),
            password: mssql_config.password.clone(),
            mssql_encrypt: Some(mssql_config.encrypt),
            mssql_trust_cert: Some(mssql_config.trust_certificate),
            postgres_sslmode: None,
            mysql_ssl_enabled: None,
        }
    }
}

impl Default for MssqlDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl DatabaseDriver for MssqlDriver {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::Mssql
    }

    async fn test_connection(&self, config: &DatabaseConfig) -> Result<bool, DatabaseError> {
        let mssql_config = self.to_mssql_config(config)?;
        self.connection_manager
            .test_connection(&mssql_config)
            .await
            .map_err(|e| DatabaseError::ConnectionFailed(e.to_string()))
    }

    async fn connect(&self, config: &DatabaseConfig) -> Result<Box<dyn Connection>, DatabaseError> {
        let mssql_config = self.to_mssql_config(config)?;
        let pool = self
            .connection_manager
            .connect(&mssql_config.id)
            .await
            .map_err(|e| DatabaseError::ConnectionFailed(e.to_string()))?;

        let connection = MssqlConnection {
            id: config.id.clone(),
            pool,
        };

        Ok(Box::new(connection))
    }

    async fn execute_query(
        &self,
        conn: &dyn Connection,
        sql: &str,
        query_id: String,
    ) -> Result<QueryResult, DatabaseError> {
        let mssql_conn = conn
            .as_any()
            .downcast_ref::<MssqlConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;

        let pool = Arc::clone(&mssql_conn.pool);
        let mut connection = pool
            .get()
            .await
            .map_err(|e| DatabaseError::PoolError(e.to_string()))?;

        let start = std::time::Instant::now();

        let query = connection.simple_query(sql).await;
        let mut result = QueryResult::new(query_id.clone());
        result.statement_text = Some(sql.to_string());

        match query {
            Ok(stream) => {
                let rows_result = stream.into_results().await;
                match rows_result {
                    Ok(result_sets) => {
                        // Process first result set for now (can support multiple later)
                        if let Some(rows) = result_sets.first() {
                            // Get column info
                            if let Some(first_row) = rows.first() {
                                result.columns = first_row
                                    .columns()
                                    .iter()
                                    .enumerate()
                                    .map(|(idx, col)| ColumnInfo {
                                        name: col.name().to_string(),
                                        data_type: Self::column_type_to_string(&col.column_type()),
                                        max_length: None,  // Not easily available from Column
                                        precision: None,
                                        scale: None,
                                        is_nullable: true,  // Default to true
                                        is_primary_key: false,
                                        is_identity: false,
                                        column_default: None,
                                        ordinal_position: idx as i32,
                                    })
                                    .collect();
                            }

                            // Get rows
                            result.rows = rows
                                .iter()
                                .map(|row| {
                                    result
                                        .columns
                                        .iter()
                                        .enumerate()
                                        .map(|(idx, _)| Self::cell_value_from_row(row, idx))
                                        .collect()
                                })
                                .collect();
                        }

                        result.row_count = result.rows.len();
                        result.is_complete = true;
                    }
                    Err(e) => {
                        result.error = Some(e.to_string());
                        result.is_complete = true;
                    }
                }
            }
            Err(e) => {
                result.error = Some(e.to_string());
                result.is_complete = true;
            }
        }

        result.execution_time_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }

    async fn cancel_query(&self, _query_id: &str) -> Result<(), DatabaseError> {
        // MS-SQL query cancellation is handled by dropping the dedicated connection
        // This is a simplified implementation
        Err(DatabaseError::QueryError("Query cancellation not yet implemented".to_string()))
    }

    async fn get_tables(&self, conn: &dyn Connection) -> Result<Vec<TableInfo>, DatabaseError> {
        let mssql_conn = conn
            .as_any()
            .downcast_ref::<MssqlConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;

        let pool = Arc::clone(&mssql_conn.pool);
        let mut connection = pool
            .get()
            .await
            .map_err(|e| DatabaseError::PoolError(e.to_string()))?;

        let query = r#"
            SELECT
                s.name as schema_name,
                t.name as table_name,
                t.type_desc as table_type
            FROM sys.tables t
            JOIN sys.schemas s ON t.schema_id = s.schema_id
            ORDER BY s.name, t.name
        "#;

        let stream = connection.simple_query(query).await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let rows = stream.into_first_result().await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let tables: Vec<TableInfo> = rows
            .iter()
            .map(|row| {
                let schema_name = row.get::<&str, _>(0).unwrap_or("").to_string();
                let table_name = row.get::<&str, _>(1).unwrap_or("").to_string();
                let table_type = row.get::<&str, _>(2).unwrap_or("BASE TABLE").to_string();

                TableInfo {
                    schema_name,
                    table_name,
                    table_type,
                    columns: Vec::new(),
                }
            })
            .collect();

        Ok(tables)
    }

    async fn get_columns(
        &self,
        conn: &dyn Connection,
        table_name: &str,
    ) -> Result<Vec<ColumnInfo>, DatabaseError> {
        let mssql_conn = conn
            .as_any()
            .downcast_ref::<MssqlConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;

        let pool = Arc::clone(&mssql_conn.pool);
        let mut connection = pool
            .get()
            .await
            .map_err(|e| DatabaseError::PoolError(e.to_string()))?;

        // Parse table_name to extract schema and table
        let parts: Vec<&str> = table_name.split('.').collect();
        let (schema, table) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            ("dbo", table_name)
        };

        // Use simple_query with properly escaped parameters
        let query = format!(r#"
            SELECT
                c.name as column_name,
                t.name as data_type,
                c.max_length,
                c.precision,
                c.scale,
                c.is_nullable,
                c.is_identity,
                c.column_default,
                c.column_id
            FROM sys.columns c
            JOIN sys.types t ON c.user_type_id = t.user_type_id
            JOIN sys.tables tbl ON c.object_id = tbl.object_id
            JOIN sys.schemas s ON tbl.schema_id = s.schema_id
            WHERE s.name = '{}' AND tbl.name = '{}'
            ORDER BY c.column_id
        "#, schema.replace("'", "''"), table.replace("'", "''"));

        let stream = connection.simple_query(&query).await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let rows = stream.into_first_result().await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let columns: Vec<ColumnInfo> = rows
            .iter()
            .map(|row| ColumnInfo {
                name: row.get::<&str, _>(0).unwrap_or("").to_string(),
                data_type: row.get::<&str, _>(1).unwrap_or("").to_string(),
                max_length: row.get::<i32, _>(2),
                precision: row.get::<i32, _>(3),
                scale: row.get::<i32, _>(4),
                is_nullable: row.get::<bool, _>(5).unwrap_or(true),
                is_primary_key: false, // Would need additional query
                is_identity: row.get::<bool, _>(6).unwrap_or(false),
                column_default: row.get::<&str, _>(7).map(|s| s.to_string()),
                ordinal_position: row.get::<i32, _>(8).unwrap_or(0),
            })
            .collect();

        Ok(columns)
    }

    async fn get_databases(&self, conn: &dyn Connection) -> Result<Vec<String>, DatabaseError> {
        let mssql_conn = conn
            .as_any()
            .downcast_ref::<MssqlConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;

        let pool = Arc::clone(&mssql_conn.pool);
        let mut connection = pool
            .get()
            .await
            .map_err(|e| DatabaseError::PoolError(e.to_string()))?;

        let query = "SELECT name FROM sys.databases WHERE state_desc = 'ONLINE' AND HAS_DBACCESS(name) = 1 ORDER BY name";
        let stream = connection.simple_query(query).await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let rows = stream.into_first_result().await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let databases: Vec<String> = rows
            .iter()
            .filter_map(|row| row.get::<&str, _>(0).map(|s| s.to_string()))
            .collect();

        Ok(databases)
    }

    async fn get_schemas(&self, conn: &dyn Connection) -> Result<Vec<String>, DatabaseError> {
        let mssql_conn = conn
            .as_any()
            .downcast_ref::<MssqlConnection>()
            .ok_or_else(|| DatabaseError::InvalidConnection)?;

        let pool = Arc::clone(&mssql_conn.pool);
        let mut connection = pool
            .get()
            .await
            .map_err(|e| DatabaseError::PoolError(e.to_string()))?;

        let query = "SELECT name FROM sys.schemas ORDER BY name";
        let stream = connection.simple_query(query).await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let rows = stream.into_first_result().await
            .map_err(|e| DatabaseError::QueryError(e.to_string()))?;

        let schemas: Vec<String> = rows
            .iter()
            .filter_map(|row| row.get::<&str, _>(0).map(|s| s.to_string()))
            .collect();

        Ok(schemas)
    }
}

impl MssqlDriver {
    /// Convert Tiberius ColumnType to string
    fn column_type_to_string(col_type: &ColumnType) -> String {
        match col_type {
            ColumnType::Int1 => "tinyint".to_string(),
            ColumnType::Int2 => "smallint".to_string(),
            ColumnType::Int4 => "int".to_string(),
            ColumnType::Int8 => "bigint".to_string(),
            ColumnType::Intn => "int".to_string(),
            ColumnType::Float4 => "real".to_string(),
            ColumnType::Float8 => "float".to_string(),
            ColumnType::Floatn => "float".to_string(),
            ColumnType::Decimaln | ColumnType::Numericn => "decimal".to_string(),
            ColumnType::Money | ColumnType::Money4 => "money".to_string(),
            ColumnType::Bit | ColumnType::Bitn => "bit".to_string(),
            ColumnType::BigVarChar | ColumnType::BigChar | ColumnType::NVarchar | ColumnType::NChar
            | ColumnType::Text | ColumnType::NText => "nvarchar".to_string(),
            ColumnType::Datetime | ColumnType::Datetime2 | ColumnType::Datetimen => "datetime".to_string(),
            ColumnType::Datetime4 => "smalldatetime".to_string(),
            ColumnType::Daten => "date".to_string(),
            ColumnType::Timen => "time".to_string(),
            ColumnType::DatetimeOffsetn => "datetimeoffset".to_string(),
            ColumnType::Guid => "uniqueidentifier".to_string(),
            ColumnType::Xml => "xml".to_string(),
            ColumnType::Null => "null".to_string(),
            _ => format!("{:?}", col_type).to_lowercase(),
        }
    }

    /// Extract cell value from row
    fn cell_value_from_row(row: &Row, idx: usize) -> CellValue {
        // Get column type first
        let col_type = row
            .columns()
            .get(idx)
            .map(|c| c.column_type())
            .unwrap_or(ColumnType::Null);

        // Check for NULL
        if row.try_get::<&str, _>(idx).ok().flatten().is_none()
            && row.try_get::<i32, _>(idx).ok().flatten().is_none()
            && row.try_get::<bool, _>(idx).ok().flatten().is_none()
        {
            // Additional NULL checks based on type
            match col_type {
                ColumnType::Null => return CellValue::Null,
                _ => {}
            }
        }

        match col_type {
            ColumnType::Null => CellValue::Null,

            // Integer types
            ColumnType::Int1 => row
                .try_get::<u8, _>(idx)
                .ok()
                .flatten()
                .map(|v| CellValue::Int(v as i64))
                .unwrap_or(CellValue::Null),
            ColumnType::Int2 => row
                .try_get::<i16, _>(idx)
                .ok()
                .flatten()
                .map(|v| CellValue::Int(v as i64))
                .unwrap_or(CellValue::Null),
            ColumnType::Int4 => row
                .try_get::<i32, _>(idx)
                .ok()
                .flatten()
                .map(|v| CellValue::Int(v as i64))
                .unwrap_or(CellValue::Null),
            ColumnType::Int8 => row
                .try_get::<i64, _>(idx)
                .ok()
                .flatten()
                .map(CellValue::Int)
                .unwrap_or(CellValue::Null),
            ColumnType::Intn => row
                .try_get::<i64, _>(idx)
                .ok()
                .flatten()
                .map(CellValue::Int)
                .or_else(|| row.try_get::<i32, _>(idx).ok().flatten().map(|v| CellValue::Int(v as i64)))
                .unwrap_or(CellValue::Null),

            // Float types
            ColumnType::Float4 => row
                .try_get::<f32, _>(idx)
                .ok()
                .flatten()
                .map(|v| CellValue::Float(v as f64))
                .unwrap_or(CellValue::Null),
            ColumnType::Float8 => row
                .try_get::<f64, _>(idx)
                .ok()
                .flatten()
                .map(CellValue::Float)
                .unwrap_or(CellValue::Null),
            ColumnType::Floatn => row
                .try_get::<f64, _>(idx)
                .ok()
                .flatten()
                .map(CellValue::Float)
                .or_else(|| row.try_get::<f32, _>(idx).ok().flatten().map(|v| CellValue::Float(v as f64)))
                .unwrap_or(CellValue::Null),

            // Decimal/Numeric types
            ColumnType::Decimaln | ColumnType::Numericn => row
                .try_get::<tiberius::numeric::Numeric, _>(idx)
                .ok()
                .flatten()
                .map(|n| CellValue::Float(f64::from(n)))
                .unwrap_or(CellValue::Null),

            // Money types
            ColumnType::Money | ColumnType::Money4 => row
                .try_get::<f64, _>(idx)
                .ok()
                .flatten()
                .map(CellValue::Float)
                .unwrap_or(CellValue::Null),

            // Boolean
            ColumnType::Bit | ColumnType::Bitn => row
                .try_get::<bool, _>(idx)
                .ok()
                .flatten()
                .map(CellValue::Bool)
                .unwrap_or(CellValue::Null),

            // String types
            ColumnType::BigVarChar | ColumnType::BigChar | ColumnType::NVarchar | ColumnType::NChar
            | ColumnType::Text | ColumnType::NText => row
                .try_get::<&str, _>(idx)
                .ok()
                .flatten()
                .map(|s| CellValue::String(s.to_string()))
                .unwrap_or(CellValue::Null),

            // Date/Time types
            ColumnType::Datetime | ColumnType::Datetime2 | ColumnType::Datetimen => row
                .try_get::<NaiveDateTime, _>(idx)
                .ok()
                .flatten()
                .map(|dt| CellValue::DateTime(dt.to_string()))
                .unwrap_or(CellValue::Null),
            ColumnType::Datetime4 => row
                .try_get::<NaiveDateTime, _>(idx)
                .ok()
                .flatten()
                .map(|dt| CellValue::DateTime(dt.to_string()))
                .unwrap_or(CellValue::Null),
            ColumnType::DatetimeOffsetn => row
                .try_get::<DateTime<Utc>, _>(idx)
                .ok()
                .flatten()
                .map(|dt| CellValue::DateTime(dt.to_rfc3339()))
                .unwrap_or(CellValue::Null),
            ColumnType::Daten => row
                .try_get::<NaiveDate, _>(idx)
                .ok()
                .flatten()
                .map(|d| CellValue::DateTime(d.to_string()))
                .unwrap_or(CellValue::Null),
            ColumnType::Timen => row
                .try_get::<NaiveTime, _>(idx)
                .ok()
                .flatten()
                .map(|t| CellValue::DateTime(t.to_string()))
                .unwrap_or(CellValue::Null),

            // Binary types
            ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image => row
                .try_get::<&[u8], _>(idx)
                .ok()
                .flatten()
                .map(|b| CellValue::Binary(b.to_vec()))
                .unwrap_or(CellValue::Null),

            // GUID
            ColumnType::Guid => row
                .try_get::<tiberius::Uuid, _>(idx)
                .ok()
                .flatten()
                .map(|u| CellValue::String(u.to_string()))
                .unwrap_or(CellValue::Null),

            // XML
            ColumnType::Xml => row
                .try_get::<&tiberius::xml::XmlData, _>(idx)
                .ok()
                .flatten()
                .map(|xml| {
                    let s = xml.to_owned().into_string();
                    CellValue::String(s)
                })
                .unwrap_or(CellValue::Null),

            // Default
            _ => CellValue::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_type_to_string() {
        assert_eq!(MssqlDriver::column_type_to_string(&ColumnType::Int4), "int");
        assert_eq!(MssqlDriver::column_type_to_string(&ColumnType::Float8), "float");
        assert_eq!(MssqlDriver::column_type_to_string(&ColumnType::NVarchar), "nvarchar");
        assert_eq!(MssqlDriver::column_type_to_string(&ColumnType::Datetime), "datetime");
    }

    #[test]
    fn test_driver_type() {
        let driver = MssqlDriver::new();
        assert_eq!(driver.database_type(), DatabaseType::Mssql);
    }

    #[test]
    fn test_config_conversion() {
        let driver = MssqlDriver::new();

        let mut config = DatabaseConfig::new("Test DB".to_string(), DatabaseType::Mssql);
        config.host = Some("localhost".to_string());
        config.port = Some(1433);
        config.database = "master".to_string();
        config.username = Some("sa".to_string());
        config.password = "password".to_string();

        let mssql_config = driver.to_mssql_config(&config);
        assert!(mssql_config.is_ok());

        let mssql_config = mssql_config.unwrap();
        assert_eq!(mssql_config.name, "Test DB");
        assert_eq!(mssql_config.host, "localhost");
        assert_eq!(mssql_config.port, 1433);
    }
}
