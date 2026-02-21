// Driver Registry (Phase 1 - Extensible Architecture)
// Manages available database drivers

use crate::db::traits::{DatabaseDriver, DatabaseType, DatabaseError};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Registry for managing database drivers
pub struct DriverRegistry {
    drivers: RwLock<HashMap<DatabaseType, Arc<dyn DatabaseDriver>>>,
}

impl DriverRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            drivers: RwLock::new(HashMap::new()),
        }
    }

    /// Register a database driver
    pub async fn register(&self, driver: Arc<dyn DatabaseDriver>) {
        let db_type = driver.database_type();
        let mut drivers = self.drivers.write().await;
        drivers.insert(db_type.clone(), driver);
        println!("[Registry] Registered driver for: {:?}", db_type);
    }

    /// Get a driver by database type
    pub async fn get_driver(
        &self,
        db_type: DatabaseType,
    ) -> Result<Arc<dyn DatabaseDriver>, DatabaseError> {
        let drivers = self.drivers.read().await;
        drivers
            .get(&db_type)
            .cloned()
            .ok_or_else(|| DatabaseError::DriverNotFound(db_type))
    }

    /// Get all registered database types
    pub async fn get_supported_types(&self) -> Vec<DatabaseType> {
        let drivers = self.drivers.read().await;
        drivers.keys().cloned().collect()
    }

    /// Check if a driver is registered for a given database type
    pub async fn has_driver(&self, db_type: DatabaseType) -> bool {
        let drivers = self.drivers.read().await;
        drivers.contains_key(&db_type)
    }

    /// Remove a driver (useful for testing or dynamic unloading)
    pub async fn unregister(&self, db_type: DatabaseType) {
        let mut drivers = self.drivers.write().await;
        drivers.remove(&db_type);
        println!("[Registry] Unregistered driver for: {:?}", db_type);
    }
}

impl Default for DriverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock driver for testing
    struct MockDriver;

    #[async_trait::async_trait]
    impl DatabaseDriver for MockDriver {
        fn database_type(&self) -> DatabaseType {
            DatabaseType::Mssql
        }

        async fn test_connection(
            &self,
            _config: &crate::db::traits::DatabaseConfig,
        ) -> Result<bool, DatabaseError> {
            Ok(false)
        }

        async fn connect(
            &self,
            _config: &crate::db::traits::DatabaseConfig,
        ) -> Result<Box<dyn crate::db::traits::Connection>, DatabaseError> {
            Err(DatabaseError::ConnectionFailed("Mock".to_string()))
        }

        async fn execute_query(
            &self,
            _conn: &dyn crate::db::traits::Connection,
            _sql: &str,
            _query_id: String,
        ) -> Result<crate::db::traits::QueryResult, DatabaseError> {
            Err(DatabaseError::QueryError("Mock".to_string()))
        }

        async fn cancel_query(&self, _query_id: &str) -> Result<(), DatabaseError> {
            Err(DatabaseError::QueryError("Mock".to_string()))
        }

        async fn get_tables(
            &self,
            _conn: &dyn crate::db::traits::Connection,
        ) -> Result<Vec<crate::db::traits::TableInfo>, DatabaseError> {
            Err(DatabaseError::SchemaError("Mock".to_string()))
        }

        async fn get_columns(
            &self,
            _conn: &dyn crate::db::traits::Connection,
            _table_name: &str,
        ) -> Result<Vec<crate::db::traits::ColumnInfo>, DatabaseError> {
            Err(DatabaseError::SchemaError("Mock".to_string()))
        }

        async fn get_databases(
            &self,
            _conn: &dyn crate::db::traits::Connection,
        ) -> Result<Vec<String>, DatabaseError> {
            Err(DatabaseError::SchemaError("Mock".to_string()))
        }

        async fn get_schemas(
            &self,
            _conn: &dyn crate::db::traits::Connection,
        ) -> Result<Vec<String>, DatabaseError> {
            Err(DatabaseError::SchemaError("Mock".to_string()))
        }
    }

    #[tokio::test]
    async fn test_register_driver() {
        let registry = DriverRegistry::new();
        let driver = Arc::new(MockDriver);

        registry.register(driver).await;

        assert!(registry.has_driver(DatabaseType::Mssql).await);
    }

    #[tokio::test]
    async fn test_get_driver() {
        let registry = DriverRegistry::new();
        let driver = Arc::new(MockDriver);

        registry.register(driver).await;

        let result = registry.get_driver(DatabaseType::Mssql).await;
        assert!(result.is_ok());
        assert!(registry.get_driver(DatabaseType::Postgresql).await.is_err());
    }

    #[tokio::test]
    async fn test_unregister_driver() {
        let registry = DriverRegistry::new();
        let driver = Arc::new(MockDriver);

        registry.register(driver.clone()).await;
        assert!(registry.has_driver(DatabaseType::Mssql).await);

        registry.unregister(DatabaseType::Mssql).await;
        assert!(!registry.has_driver(DatabaseType::Mssql).await);
    }

    #[tokio::test]
    async fn test_get_supported_types() {
        let registry = DriverRegistry::new();
        let driver = Arc::new(MockDriver);

        registry.register(driver).await;

        let types = registry.get_supported_types().await;
        assert_eq!(types.len(), 1);
        assert_eq!(types[0], DatabaseType::Mssql);
    }
}
