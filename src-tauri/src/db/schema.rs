// Schema Metadata (T024)
// Driver-neutral schema model plus caching. Catalog queries live in each driver.

use crate::db::error::ConnectionError;
use crate::db::manager::ConnectionManager;
use crate::storage::DatabaseManager;
use serde::{Deserialize, Serialize};
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
    connection_manager: Arc<ConnectionManager>,
    db_manager: Arc<DatabaseManager>,
}

impl SchemaMetadataManager {
    pub fn new(
        connection_manager: Arc<ConnectionManager>,
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
        let (driver, config) = self.connection_manager.resolve(connection_id).await?;
        let schema_info = driver
            .fetch_schema(&config, database, schema_filter)
            .await?;

        // Cache the result
        let _ = self
            .db_manager
            .save_schema(connection_id, database, &schema_info);

        Ok(schema_info)
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

        let (driver, config) = self.connection_manager.resolve(connection_id).await?;
        driver
            .fetch_table_columns(&config, database, schema_name, table_name)
            .await
    }
}
