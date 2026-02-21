// SQLite Connection Manager
// Manages SQLite file-based connections (no pooling needed - single writer)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::db::connection::{ConnectionError, ConnectionInfo};
use crate::db::drivers::sqlite::SqliteDriver;
use crate::db::traits::{DatabaseConfig, DatabaseDriver};

/// Connection configuration for SQLite
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteConfig {
    pub id: String,
    pub name: String,
    /// Absolute path to the SQLite database file
    pub path: String,
    pub space_id: Option<String>,
}

impl From<&SqliteConfig> for ConnectionInfo {
    fn from(config: &SqliteConfig) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            host: "localhost".to_string(),
            port: 0,
            database: config.path.clone(),
            username: String::new(),
            trust_certificate: false,
            encrypt: false,
            space_id: config.space_id.clone(),
            is_connected: false,
        }
    }
}

/// Manages multiple SQLite connections
/// SQLite is file-based so "pooling" is just opening connections on demand
pub struct SqliteConnectionManager {
    /// Map of connection ID -> config
    configs: RwLock<HashMap<String, SqliteConfig>>,
    /// Set of currently "connected" IDs (opened at least once)
    connected: RwLock<std::collections::HashSet<String>>,
}

impl SqliteConnectionManager {
    pub fn new() -> Self {
        Self {
            configs: RwLock::new(HashMap::new()),
            connected: RwLock::new(std::collections::HashSet::new()),
        }
    }

    pub async fn add_connection(&self, config: SqliteConfig) -> Result<String, ConnectionError> {
        let id = config.id.clone();
        let mut configs = self.configs.write().await;
        configs.insert(id.clone(), config);
        Ok(id)
    }

    pub async fn get_config(&self, id: &str) -> Option<SqliteConfig> {
        let configs = self.configs.read().await;
        configs.get(id).cloned()
    }

    /// "Connect" - validate the file is accessible
    pub async fn connect(&self, id: &str) -> Result<(), ConnectionError> {
        let config = self.get_config(id).await
            .ok_or_else(|| ConnectionError::NotFound(id.to_string()))?;

        // Validate that the path is accessible by attempting to open
        let driver = SqliteDriver::new();
        let mut db_config = DatabaseConfig::new(config.name.clone(), crate::db::traits::DatabaseType::Sqlite);
        db_config.id = id.to_string();
        db_config.database = config.path.clone();

        driver.test_connection(&db_config)
            .await
            .map_err(|e: crate::db::traits::DatabaseError| ConnectionError::ConnectionFailed(e.to_string()))?;

        let mut connected = self.connected.write().await;
        connected.insert(id.to_string());
        Ok(())
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), ConnectionError> {
        let mut connected = self.connected.write().await;
        connected.remove(id);
        Ok(())
    }

    pub async fn is_healthy(&self, id: &str) -> bool {
        let connected = self.connected.read().await;
        if !connected.contains(id) {
            return false;
        }

        // Try to open the file to verify it's still accessible
        if let Some(config) = self.get_config(id).await {
            std::path::Path::new(&config.path).exists()
        } else {
            false
        }
    }

    pub async fn get_pool(&self, id: &str) -> Option<()> {
        let connected = self.connected.read().await;
        if connected.contains(id) { Some(()) } else { None }
    }

    pub async fn list_connections(&self) -> Vec<ConnectionInfo> {
        let configs = self.configs.read().await;
        let connected = self.connected.read().await;
        configs.values().map(|c| {
            let mut info = ConnectionInfo::from(c);
            info.is_connected = connected.contains(&c.id);
            info
        }).collect()
    }

    pub async fn get_connections_by_space(&self, space_id: &str) -> Vec<ConnectionInfo> {
        let configs = self.configs.read().await;
        let connected = self.connected.read().await;
        configs.values()
            .filter(|c| c.space_id.as_deref() == Some(space_id))
            .map(|c| {
                let mut info = ConnectionInfo::from(c);
                info.is_connected = connected.contains(&c.id);
                info
            })
            .collect()
    }

    pub async fn get_connection(&self, id: &str) -> Option<ConnectionInfo> {
        let configs = self.configs.read().await;
        let connected = self.connected.read().await;
        configs.get(id).map(|c| {
            let mut info = ConnectionInfo::from(c);
            info.is_connected = connected.contains(id);
            info
        })
    }

    /// Build a DatabaseConfig for executing queries against this connection
    pub async fn get_database_config(&self, id: &str) -> Option<DatabaseConfig> {
        let config = self.get_config(id).await?;
        let mut db_config = DatabaseConfig::new(
            config.name,
            crate::db::traits::DatabaseType::Sqlite,
        );
        db_config.id = config.id;
        db_config.database = config.path;
        db_config.space_id = config.space_id;
        Some(db_config)
    }
}

impl Default for SqliteConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}
