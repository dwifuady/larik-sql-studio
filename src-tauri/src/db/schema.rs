// Schema Metadata Fetching (T024)
// Queries SQL Server system catalogs for database schema information

use crate::db::connection::{ConnectionError, MssqlConnectionManager};
use crate::storage::DatabaseManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Represents a column in a table or view
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub max_length: Option<i32>,
    pub precision: Option<i32>,
    pub scale: Option<i32>,
    pub is_nullable: bool,
    pub is_primary_key: bool,
    pub is_identity: bool,
    pub is_computed: bool,
    pub column_default: Option<String>,
    pub ordinal_position: i32,
}

/// Represents a table or view in the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub schema_name: String,
    pub table_name: String,
    pub table_type: String, // "BASE TABLE" or "VIEW"
    pub columns: Vec<ColumnInfo>,
}

/// Represents a foreign key relationship between two table columns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipInfo {
    pub constraint_name: String,
    pub ordinal_position: i32,
    pub source_schema_name: String,
    pub source_table_name: String,
    pub source_column_name: String,
    pub target_schema_name: String,
    pub target_table_name: String,
    pub target_column_name: String,
}

/// Represents a parameter of a stored procedure or function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterInfo {
    pub name: String,
    pub data_type: String,
    pub max_length: Option<i32>,
    pub precision: Option<i32>,
    pub scale: Option<i32>,
    pub parameter_mode: String, // "IN", "OUT", "INOUT"
    pub ordinal_position: i32,
    pub has_default: bool,
}

/// Represents a stored procedure or function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineInfo {
    pub schema_name: String,
    pub routine_name: String,
    pub routine_type: String, // "PROCEDURE" or "FUNCTION"
    pub return_type: Option<String>,
    pub parameters: Vec<ParameterInfo>,
}

/// Complete schema information for a database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaInfo {
    pub database_name: String,
    pub schemas: Vec<String>,
    pub tables: Vec<TableInfo>,
    #[serde(default)]
    pub relationships: Vec<RelationshipInfo>,
    pub routines: Vec<RoutineInfo>,
    pub fetched_at: String,
}

/// Manages schema metadata caching per connection/database
pub struct SchemaMetadataManager {
    connection_manager: Arc<MssqlConnectionManager>,
    db_manager: Arc<DatabaseManager>,
}

impl SchemaMetadataManager {
    pub fn new(
        connection_manager: Arc<MssqlConnectionManager>,
        db_manager: Arc<DatabaseManager>,
    ) -> Self {
        Self {
            connection_manager,
            db_manager,
        }
    }

    /// Get schema info from cache
    pub async fn get_cached_schema(
        &self,
        connection_id: &str,
        database: &str,
    ) -> Option<SchemaInfo> {
        self.db_manager
            .get_schema(connection_id, database)
            .unwrap_or(None)
    }

    /// Clear schema cache for a connection/database
    pub async fn invalidate_cache(&self, connection_id: &str, database: Option<&str>) {
        let _ = self.db_manager.clear_schema(connection_id, database);
    }

    /// Fetch and cache schema information for a database
    pub async fn fetch_schema(
        &self,
        connection_id: &str,
        database: &str,
        schema_filter: Option<&str>,
    ) -> Result<SchemaInfo, ConnectionError> {
        // Use a dedicated connection so the `USE` does not pollute a pooled connection
        let mut conn = self
            .connection_manager
            .create_dedicated_connection(connection_id)
            .await?;

        // Switch to the target database — validated + bracket-quoted
        let use_db_query = format!(
            "USE {};",
            crate::db::ident::resolve_database_name(database)
                .map_err(|e| ConnectionError::QueryError(e))?
        );
        conn.simple_query(&use_db_query).await?;

        // Fetch schemas
        let schemas = self.fetch_schemas(&mut conn).await?;

        // Fetch tables and views
        let tables = self
            .fetch_tables_and_views(&mut conn, schema_filter)
            .await?;

        // Fetch foreign key relationships for join suggestions
        let relationships = self.fetch_relationships(&mut conn, schema_filter).await?;

        // Fetch routines (stored procedures and functions)
        let routines = self.fetch_routines(&mut conn, schema_filter).await?;

        let schema_info = SchemaInfo {
            database_name: database.to_string(),
            schemas,
            tables,
            relationships,
            routines,
            fetched_at: chrono::Utc::now().to_rfc3339(),
        };

        // Cache the result
        let _ = self
            .db_manager
            .save_schema(connection_id, database, &schema_info);

        Ok(schema_info)
    }

    /// Fetch foreign key relationships between tables
    async fn fetch_relationships(
        &self,
        conn: &mut tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>,
        schema_filter: Option<&str>,
    ) -> Result<Vec<RelationshipInfo>, ConnectionError> {
        if let Some(s) = schema_filter {
            if !crate::db::ident::is_safe_ident(s) {
                return Err(ConnectionError::QueryError("Invalid identifier".into()));
            }
        }
        let schema_condition = schema_filter
            .map(|s| {
                format!(
                    "AND (src_schema.name = '{}' OR tgt_schema.name = '{}')",
                    s.replace('\'', "''"),
                    s.replace('\'', "''")
                )
            })
            .unwrap_or_default();

        let query = format!(
            r#"
            SELECT
                fk.name AS constraint_name,
                fkc.constraint_column_id AS ordinal_position,
                src_schema.name AS source_schema_name,
                src_table.name AS source_table_name,
                src_column.name AS source_column_name,
                tgt_schema.name AS target_schema_name,
                tgt_table.name AS target_table_name,
                tgt_column.name AS target_column_name
            FROM sys.foreign_keys fk
            JOIN sys.foreign_key_columns fkc
                ON fk.object_id = fkc.constraint_object_id
            JOIN sys.tables src_table
                ON fkc.parent_object_id = src_table.object_id
            JOIN sys.schemas src_schema
                ON src_table.schema_id = src_schema.schema_id
            JOIN sys.columns src_column
                ON fkc.parent_object_id = src_column.object_id
                AND fkc.parent_column_id = src_column.column_id
            JOIN sys.tables tgt_table
                ON fkc.referenced_object_id = tgt_table.object_id
            JOIN sys.schemas tgt_schema
                ON tgt_table.schema_id = tgt_schema.schema_id
            JOIN sys.columns tgt_column
                ON fkc.referenced_object_id = tgt_column.object_id
                AND fkc.referenced_column_id = tgt_column.column_id
            WHERE 1=1 {}
            ORDER BY fk.name, fkc.constraint_column_id
        "#,
            schema_condition
        );

        let stream = conn.simple_query(&query).await?;
        let rows = stream.into_first_result().await?;

        let relationships = rows
            .iter()
            .filter_map(|row| {
                Some(RelationshipInfo {
                    constraint_name: row.get::<&str, _>(0)?.to_string(),
                    ordinal_position: row.get::<i32, _>(1).unwrap_or(0),
                    source_schema_name: row.get::<&str, _>(2)?.to_string(),
                    source_table_name: row.get::<&str, _>(3)?.to_string(),
                    source_column_name: row.get::<&str, _>(4)?.to_string(),
                    target_schema_name: row.get::<&str, _>(5)?.to_string(),
                    target_table_name: row.get::<&str, _>(6)?.to_string(),
                    target_column_name: row.get::<&str, _>(7)?.to_string(),
                })
            })
            .collect();

        Ok(relationships)
    }

    /// Fetch all schema names in the database
    async fn fetch_schemas(
        &self,
        conn: &mut tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>,
    ) -> Result<Vec<String>, ConnectionError> {
        let query = r#"
            SELECT schema_name
            FROM INFORMATION_SCHEMA.SCHEMATA
            WHERE schema_name NOT IN ('guest', 'INFORMATION_SCHEMA', 'sys')
            ORDER BY schema_name
        "#;

        let stream = conn.simple_query(query).await?;
        let rows = stream.into_first_result().await?;

        let schemas: Vec<String> = rows
            .iter()
            .filter_map(|row| row.get::<&str, _>(0).map(|s| s.to_string()))
            .collect();

        Ok(schemas)
    }

    /// Fetch tables and views with their columns
    async fn fetch_tables_and_views(
        &self,
        conn: &mut tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>,
        schema_filter: Option<&str>,
    ) -> Result<Vec<TableInfo>, ConnectionError> {
        if let Some(s) = schema_filter {
            if !crate::db::ident::is_safe_ident(s) {
                return Err(ConnectionError::QueryError("Invalid identifier".into()));
            }
        }
        // First, fetch all tables and views
        let schema_condition = schema_filter
            .map(|s| format!("AND t.TABLE_SCHEMA = '{}'", s.replace('\'', "''")))
            .unwrap_or_default();

        let tables_query = format!(
            r#"
            SELECT
                t.TABLE_SCHEMA,
                t.TABLE_NAME,
                t.TABLE_TYPE
            FROM INFORMATION_SCHEMA.TABLES t
            WHERE t.TABLE_TYPE IN ('BASE TABLE', 'VIEW')
            {}
            ORDER BY t.TABLE_SCHEMA, t.TABLE_NAME
        "#,
            schema_condition
        );

        let stream = conn.simple_query(&tables_query).await?;
        let table_rows = stream.into_first_result().await?;

        // Collect table info
        let mut tables: Vec<TableInfo> = table_rows
            .iter()
            .filter_map(|row| {
                let schema_name = row.get::<&str, _>(0)?.to_string();
                let table_name = row.get::<&str, _>(1)?.to_string();
                let table_type = row.get::<&str, _>(2)?.to_string();
                Some(TableInfo {
                    schema_name,
                    table_name,
                    table_type,
                    columns: vec![],
                })
            })
            .collect();

        // Fetch columns for all tables
        let columns_query = format!(
            r#"
            SELECT
                c.TABLE_SCHEMA,
                c.TABLE_NAME,
                c.COLUMN_NAME,
                c.DATA_TYPE,
                c.CHARACTER_MAXIMUM_LENGTH,
                c.NUMERIC_PRECISION,
                c.NUMERIC_SCALE,
                c.IS_NULLABLE,
                c.COLUMN_DEFAULT,
                c.ORDINAL_POSITION,
                CASE WHEN pk.COLUMN_NAME IS NOT NULL THEN 1 ELSE 0 END AS IS_PRIMARY_KEY,
                COLUMNPROPERTY(OBJECT_ID(c.TABLE_SCHEMA + '.' + c.TABLE_NAME), c.COLUMN_NAME, 'IsIdentity') AS IS_IDENTITY,
                COLUMNPROPERTY(OBJECT_ID(c.TABLE_SCHEMA + '.' + c.TABLE_NAME), c.COLUMN_NAME, 'IsComputed') AS IS_COMPUTED
            FROM INFORMATION_SCHEMA.COLUMNS c
            LEFT JOIN (
                SELECT
                    ku.TABLE_SCHEMA,
                    ku.TABLE_NAME,
                    ku.COLUMN_NAME
                FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
                JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE ku
                    ON tc.CONSTRAINT_NAME = ku.CONSTRAINT_NAME
                    AND tc.TABLE_SCHEMA = ku.TABLE_SCHEMA
                WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY'
            ) pk ON c.TABLE_SCHEMA = pk.TABLE_SCHEMA
                AND c.TABLE_NAME = pk.TABLE_NAME
                AND c.COLUMN_NAME = pk.COLUMN_NAME
            WHERE 1=1 {}
            ORDER BY c.TABLE_SCHEMA, c.TABLE_NAME, c.ORDINAL_POSITION
        "#,
            schema_filter
                .map(|s| format!("AND c.TABLE_SCHEMA = '{}'", s.replace('\'', "''")))
                .unwrap_or_default()
        );

        let stream = conn.simple_query(&columns_query).await?;
        let column_rows = stream.into_first_result().await?;

        // Group columns by table
        let mut columns_by_table: HashMap<(String, String), Vec<ColumnInfo>> = HashMap::new();

        for row in column_rows.iter() {
            let schema_name = match row.get::<&str, _>(0) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let table_name = match row.get::<&str, _>(1) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let column_name = match row.get::<&str, _>(2) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let data_type = match row.get::<&str, _>(3) {
                Some(s) => s.to_string(),
                None => "unknown".to_string(),
            };
            let max_length = row.get::<i32, _>(4);
            // NUMERIC_PRECISION can be tinyint (u8) or smallint (i16)
            let precision = row
                .try_get::<u8, _>(5)
                .ok()
                .flatten()
                .map(|v| v as i32)
                .or_else(|| row.try_get::<i16, _>(5).ok().flatten().map(|v| v as i32));
            // NUMERIC_SCALE can be tinyint (u8) or int
            let scale = row
                .try_get::<u8, _>(6)
                .ok()
                .flatten()
                .map(|v| v as i32)
                .or_else(|| row.try_get::<i32, _>(6).ok().flatten());
            let is_nullable = row.get::<&str, _>(7).map(|s| s == "YES").unwrap_or(true);
            let column_default = row.get::<&str, _>(8).map(|s| s.to_string());
            let ordinal_position = row.get::<i32, _>(9).unwrap_or(0);
            let is_primary_key = row.get::<i32, _>(10).map(|v| v == 1).unwrap_or(false);
            let is_identity = row.get::<i32, _>(11).map(|v| v == 1).unwrap_or(false);
            let is_computed = row.get::<i32, _>(12).map(|v| v == 1).unwrap_or(false);

            let column = ColumnInfo {
                name: column_name,
                data_type,
                max_length,
                precision,
                scale,
                is_nullable,
                is_primary_key,
                is_identity,
                is_computed,
                column_default,
                ordinal_position,
            };

            columns_by_table
                .entry((schema_name, table_name))
                .or_default()
                .push(column);
        }

        // Assign columns to tables
        for table in &mut tables {
            if let Some(cols) =
                columns_by_table.remove(&(table.schema_name.clone(), table.table_name.clone()))
            {
                table.columns = cols;
            }
        }

        Ok(tables)
    }

    /// Fetch stored procedures and functions with their parameters
    async fn fetch_routines(
        &self,
        conn: &mut tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>,
        schema_filter: Option<&str>,
    ) -> Result<Vec<RoutineInfo>, ConnectionError> {
        if let Some(s) = schema_filter {
            if !crate::db::ident::is_safe_ident(s) {
                return Err(ConnectionError::QueryError("Invalid identifier".into()));
            }
        }
        let schema_condition = schema_filter
            .map(|s| format!("AND ROUTINE_SCHEMA = '{}'", s.replace('\'', "''")))
            .unwrap_or_default();

        // First, fetch all routines
        let routines_query = format!(
            r#"
            SELECT
                ROUTINE_SCHEMA,
                ROUTINE_NAME,
                ROUTINE_TYPE,
                DATA_TYPE
            FROM INFORMATION_SCHEMA.ROUTINES
            WHERE ROUTINE_TYPE IN ('PROCEDURE', 'FUNCTION')
            {}
            ORDER BY ROUTINE_SCHEMA, ROUTINE_NAME
        "#,
            schema_condition
        );

        let stream = conn.simple_query(&routines_query).await?;
        let routine_rows = stream.into_first_result().await?;

        let mut routines: Vec<RoutineInfo> = routine_rows
            .iter()
            .filter_map(|row| {
                let schema_name = row.get::<&str, _>(0)?.to_string();
                let routine_name = row.get::<&str, _>(1)?.to_string();
                let routine_type = row.get::<&str, _>(2)?.to_string();
                let return_type = row.get::<&str, _>(3).map(|s| s.to_string());
                Some(RoutineInfo {
                    schema_name,
                    routine_name,
                    routine_type,
                    return_type,
                    parameters: vec![],
                })
            })
            .collect();

        // Fetch parameters for all routines
        let params_query = format!(
            r#"
            SELECT
                SPECIFIC_SCHEMA,
                SPECIFIC_NAME,
                PARAMETER_NAME,
                DATA_TYPE,
                CHARACTER_MAXIMUM_LENGTH,
                NUMERIC_PRECISION,
                NUMERIC_SCALE,
                PARAMETER_MODE,
                ORDINAL_POSITION
            FROM INFORMATION_SCHEMA.PARAMETERS
            WHERE PARAMETER_NAME IS NOT NULL
            {}
            ORDER BY SPECIFIC_SCHEMA, SPECIFIC_NAME, ORDINAL_POSITION
        "#,
            schema_filter
                .map(|s| format!("AND SPECIFIC_SCHEMA = '{}'", s.replace('\'', "''")))
                .unwrap_or_default()
        );

        let stream = conn.simple_query(&params_query).await?;
        let param_rows = stream.into_first_result().await?;

        // Group parameters by routine
        let mut params_by_routine: HashMap<(String, String), Vec<ParameterInfo>> = HashMap::new();

        for row in param_rows.iter() {
            let schema_name = match row.get::<&str, _>(0) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let routine_name = match row.get::<&str, _>(1) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let param_name = match row.get::<&str, _>(2) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let data_type = match row.get::<&str, _>(3) {
                Some(s) => s.to_string(),
                None => "unknown".to_string(),
            };
            let max_length = row.get::<i32, _>(4);
            let precision = row
                .try_get::<u8, _>(5)
                .ok()
                .flatten()
                .map(|v| v as i32)
                .or_else(|| row.try_get::<i16, _>(5).ok().flatten().map(|v| v as i32));
            let scale = row
                .try_get::<u8, _>(6)
                .ok()
                .flatten()
                .map(|v| v as i32)
                .or_else(|| row.try_get::<i32, _>(6).ok().flatten());
            let parameter_mode = row.get::<&str, _>(7).unwrap_or("IN").to_string();
            let ordinal_position = row.get::<i32, _>(8).unwrap_or(0);

            let param = ParameterInfo {
                name: param_name,
                data_type,
                max_length,
                precision,
                scale,
                parameter_mode,
                ordinal_position,
                has_default: false, // SQL Server doesn't expose this in INFORMATION_SCHEMA
            };

            params_by_routine
                .entry((schema_name, routine_name))
                .or_default()
                .push(param);
        }

        // Assign parameters to routines
        for routine in &mut routines {
            if let Some(params) = params_by_routine
                .remove(&(routine.schema_name.clone(), routine.routine_name.clone()))
            {
                routine.parameters = params;
            }
        }

        Ok(routines)
    }

    /// Get columns for a specific table
    pub async fn get_table_columns(
        &self,
        connection_id: &str,
        database: &str,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<ColumnInfo>, ConnectionError> {
        // Try cache first
        if let Some(schema) = self.get_cached_schema(connection_id, database).await {
            if let Some(table) = schema
                .tables
                .iter()
                .find(|t| t.schema_name == schema_name && t.table_name == table_name)
            {
                return Ok(table.columns.clone());
            }
        }

        // Fetch from database — dedicated connection so `USE` does not pollute the pool
        let mut conn = self
            .connection_manager
            .create_dedicated_connection(connection_id)
            .await?;

        // Switch to the target database — validated + bracket-quoted
        let use_db_query = format!(
            "USE {};",
            crate::db::ident::resolve_database_name(database)
                .map_err(|e| ConnectionError::QueryError(e))?
        );
        conn.simple_query(&use_db_query).await?;

        // Validate schema/table — they must be strict identifiers for the string-literal
        // contexts below (WHERE … = '…'); bracket-quoted OBJECT_ID target is built separately.
        if !crate::db::ident::is_safe_ident(schema_name)
            || !crate::db::ident::is_safe_ident(table_name)
        {
            return Err(ConnectionError::QueryError("Invalid identifier".into()));
        }
        let target = crate::db::ident::qualified_object_name(schema_name, table_name)
            .map_err(|e| ConnectionError::QueryError(e))?;
        let s_esc = schema_name.replace('\'', "''");
        let t_esc = table_name.replace('\'', "''");

        let query = format!(
            r#"
            SELECT
                c.COLUMN_NAME,
                c.DATA_TYPE,
                c.CHARACTER_MAXIMUM_LENGTH,
                c.NUMERIC_PRECISION,
                c.NUMERIC_SCALE,
                c.IS_NULLABLE,
                c.COLUMN_DEFAULT,
                c.ORDINAL_POSITION,
                CASE WHEN pk.COLUMN_NAME IS NOT NULL THEN 1 ELSE 0 END AS IS_PRIMARY_KEY,
                COLUMNPROPERTY(OBJECT_ID(N'{target}'), c.COLUMN_NAME, 'IsIdentity') AS IS_IDENTITY,
                COLUMNPROPERTY(OBJECT_ID(N'{target}'), c.COLUMN_NAME, 'IsComputed') AS IS_COMPUTED
            FROM INFORMATION_SCHEMA.COLUMNS c
            LEFT JOIN (
                SELECT ku.COLUMN_NAME
                FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
                JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE ku
                    ON tc.CONSTRAINT_NAME = ku.CONSTRAINT_NAME
                    AND tc.TABLE_SCHEMA = ku.TABLE_SCHEMA
                WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY'
                    AND tc.TABLE_SCHEMA = '{s_esc}'
                    AND tc.TABLE_NAME = '{t_esc}'
            ) pk ON c.COLUMN_NAME = pk.COLUMN_NAME
            WHERE c.TABLE_SCHEMA = '{s_esc}' AND c.TABLE_NAME = '{t_esc}'
            ORDER BY c.ORDINAL_POSITION
        "#
        );

        let stream = conn.simple_query(&query).await?;
        let rows = stream.into_first_result().await?;

        let columns: Vec<ColumnInfo> = rows
            .iter()
            .filter_map(|row| {
                let name = row.get::<&str, _>(0)?.to_string();
                let data_type = row.get::<&str, _>(1)?.to_string();
                let max_length = row.get::<i32, _>(2);
                // NUMERIC_PRECISION can be tinyint (u8) or smallint (i16)
                let precision = row
                    .try_get::<u8, _>(3)
                    .ok()
                    .flatten()
                    .map(|v| v as i32)
                    .or_else(|| row.try_get::<i16, _>(3).ok().flatten().map(|v| v as i32));
                // NUMERIC_SCALE can be tinyint (u8) or int
                let scale = row
                    .try_get::<u8, _>(4)
                    .ok()
                    .flatten()
                    .map(|v| v as i32)
                    .or_else(|| row.try_get::<i32, _>(4).ok().flatten());
                let is_nullable = row.get::<&str, _>(5).map(|s| s == "YES").unwrap_or(true);
                let column_default = row.get::<&str, _>(6).map(|s| s.to_string());
                let ordinal_position = row.get::<i32, _>(7).unwrap_or(0);
                let is_primary_key = row.get::<i32, _>(8).map(|v| v == 1).unwrap_or(false);
                let is_identity = row.get::<i32, _>(9).map(|v| v == 1).unwrap_or(false);
                let is_computed = row.get::<i32, _>(10).map(|v| v == 1).unwrap_or(false);

                Some(ColumnInfo {
                    name,
                    data_type,
                    max_length,
                    precision,
                    scale,
                    is_nullable,
                    is_primary_key,
                    is_identity,
                    is_computed,
                    column_default,
                    ordinal_position,
                })
            })
            .collect();

        Ok(columns)
    }
}

impl Default for SchemaMetadataManager {
    fn default() -> Self {
        panic!("SchemaMetadataManager requires a connection manager to be initialized")
    }
}
