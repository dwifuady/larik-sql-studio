use bb8::Pool;
use bb8_postgres::PostgresConnectionManager as Bb8PostgresManager;
use bb8::ManageConnection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_postgres::NoTls;
use crate::db::connection::{ConnectionError, ConnectionInfo};

/// Connection configuration for PostgreSQL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub sslmode: String,
    pub space_id: Option<String>,
}

impl From<&PostgresConfig> for ConnectionInfo {
    fn from(config: &PostgresConfig) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            host: config.host.clone(),
            port: config.port,
            database: config.database.clone(),
            username: config.username.clone(),
            trust_certificate: false, // Not used for Postgres in this context
            encrypt: config.sslmode != "disable", // Map valid SSL modes to encrypt
            space_id: config.space_id.clone(),
            is_connected: false,
        }
    }
}

pub type PostgresPool = Pool<Bb8PostgresManager<NoTls>>;

/// Manages multiple PostgreSQL connections with pooling
pub struct PostgresConnectionManager {
    /// Map of connection ID -> connection pool
    pools: RwLock<HashMap<String, Arc<PostgresPool>>>,
    /// Map of connection ID -> connection config
    configs: RwLock<HashMap<String, PostgresConfig>>,
}

impl PostgresConnectionManager {
    pub fn new() -> Self {
        Self {
            pools: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
        }
    }

    pub async fn add_connection(&self, config: PostgresConfig) -> Result<String, ConnectionError> {
        let id = config.id.clone();
        let mut configs = self.configs.write().await;
        configs.insert(id.clone(), config);
        Ok(id)
    }

    pub async fn test_connection(&self, config: &PostgresConfig) -> Result<bool, ConnectionError> {
        let manager = Bb8PostgresManager::new_from_stringlike(
            Self::build_connection_string(config), 
            NoTls
        ).map_err(|e| ConnectionError::ConfigError(e.to_string()))?;

        // Try to create a single connection
        let _conn = manager.connect().await
            .map_err(|e: tokio_postgres::Error| ConnectionError::ConnectionFailed(e.to_string()))?;
            
        Ok(true)
    }

    pub async fn connect(&self, connection_id: &str) -> Result<Arc<PostgresPool>, ConnectionError> {
        // Check if pool already exists
        {
            let pools = self.pools.read().await;
            if let Some(pool) = pools.get(connection_id) {
                return Ok(Arc::clone(pool));
            }
        }

        // Get config
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id)
                .ok_or_else(|| ConnectionError::NotFound(connection_id.to_string()))?
                .clone()
        };

        // Create new pool
        let manager = Bb8PostgresManager::new_from_stringlike(
            Self::build_connection_string(&config), 
            NoTls
        ).map_err(|e| ConnectionError::ConfigError(e.to_string()))?;
        
        let pool = Pool::builder()
            .max_size(10)
            .min_idle(Some(1))
            .build(manager)
            .await
            .map_err(|e| ConnectionError::PoolError(e.to_string()))?;

        let pool = Arc::new(pool);
        
        // Store pool
        {
            let mut pools = self.pools.write().await;
            pools.insert(connection_id.to_string(), Arc::clone(&pool));
        }

        Ok(pool)
    }

    pub async fn disconnect(&self, connection_id: &str) -> Result<(), ConnectionError> {
        let mut pools = self.pools.write().await;
        pools.remove(connection_id);
        Ok(())
    }

    pub async fn get_pool(&self, connection_id: &str) -> Option<Arc<PostgresPool>> {
        let pools = self.pools.read().await;
        pools.get(connection_id).map(Arc::clone)
    }
    
    pub async fn list_connections(&self) -> Vec<ConnectionInfo> {
        let configs = self.configs.read().await;
        let pools = self.pools.read().await;
        
        configs.values().map(|config| {
            let mut info = ConnectionInfo::from(config);
            info.is_connected = pools.contains_key(&config.id);
            info
        }).collect()
    }

    pub async fn get_connections_by_space(&self, space_id: &str) -> Vec<ConnectionInfo> {
        let configs = self.configs.read().await;
        let pools = self.pools.read().await;
        
        configs.values()
            .filter(|config| config.space_id.as_deref() == Some(space_id))
            .map(|config| {
                let mut info = ConnectionInfo::from(config);
                info.is_connected = pools.contains_key(&config.id);
                info
            })
            .collect()
    }
    
    pub async fn get_connection(&self, id: &str) -> Option<ConnectionInfo> {
        let configs = self.configs.read().await;
        let pools = self.pools.read().await;
        
        configs.get(id).map(|config| {
            let mut info = ConnectionInfo::from(config);
            info.is_connected = pools.contains_key(&config.id);
            info
        })
    }
    
    pub async fn remove_connection(&self, connection_id: &str) -> Result<(), ConnectionError> {
        self.disconnect(connection_id).await?;
        let mut configs = self.configs.write().await;
        configs.remove(connection_id);
         Ok(())
    }
    
    pub async fn is_healthy(&self, connection_id: &str) -> bool {
        let pool = {
            let pools = self.pools.read().await;
            pools.get(connection_id).map(Arc::clone)
        };
        
        if let Some(pool) = pool {
             match pool.get().await {
                 Ok(_) => true,
                 Err(_) => false,
             }
        } else {
             false
        }
    }

    // Helper to build connection string
    fn build_connection_string(config: &PostgresConfig) -> String {
        format!(
            "host={} port={} dbname={} user={} password={} sslmode={}",
            config.host, config.port, config.database, config.username, config.password, config.sslmode
        )
    }
}
