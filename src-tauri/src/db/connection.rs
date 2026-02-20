// MS-SQL Connection Management (T015, T016)
// Handles database connections using tiberius with connection pooling

use bb8::Pool;
use bb8_tiberius::ConnectionManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tiberius::{AuthMethod, Config, EncryptionLevel};
use tokio_util::compat::TokioAsyncWriteCompatExt;

/// Connection configuration for MS-SQL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(skip_serializing)] // Don't serialize password
    pub password: String,
    pub trust_certificate: bool,
    pub encrypt: bool,
    pub space_id: Option<String>,
}

impl ConnectionConfig {
    pub fn new(
        name: String,
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            host,
            port,
            database,
            username,
            password,
            trust_certificate: true,
            encrypt: false,
            space_id: None,
        }
    }

    /// Create a tiberius Config from this ConnectionConfig
    pub fn to_tiberius_config(&self) -> Result<Config, ConnectionError> {
        let mut config = Config::new();
        config.host(&self.host);
        config.port(self.port);
        config.database(&self.database);
        config.authentication(AuthMethod::sql_server(&self.username, &self.password));
        
        if self.trust_certificate {
            config.trust_cert();
        }
        
        config.encryption(if self.encrypt {
            EncryptionLevel::Required
        } else {
            EncryptionLevel::Off
        });

        Ok(config)
    }
}

/// Serializable version of ConnectionConfig (without password for frontend)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub trust_certificate: bool,
    pub encrypt: bool,
    pub space_id: Option<String>,
    pub is_connected: bool,
}

impl From<&ConnectionConfig> for ConnectionInfo {
    fn from(config: &ConnectionConfig) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            host: config.host.clone(),
            port: config.port,
            database: config.database.clone(),
            username: config.username.clone(),
            trust_certificate: config.trust_certificate,
            encrypt: config.encrypt,
            space_id: config.space_id.clone(),
            is_connected: false,
        }
    }
}

/// Connection errors
#[derive(Error, Debug)]
pub enum ConnectionError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    
    #[error("Connection not found: {0}")]
    NotFound(String),
    
    #[error("Connection pool error: {0}")]
    PoolError(String),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    #[error("Query execution error: {0}")]
    QueryError(String),
    
    #[error("Password expired. Please change your password using another tool properly.")]
    PasswordExpired,
    
    #[error("Timeout error")]
    Timeout,
}

impl From<tiberius::error::Error> for ConnectionError {
    fn from(err: tiberius::error::Error) -> Self {
        // Check for password expired error (usually 18488)
        if let tiberius::error::Error::Server(e) = &err {
            if e.code() == 18488 {
                return ConnectionError::PasswordExpired;
            }
        }
        ConnectionError::ConnectionFailed(err.to_string())
    }
}

impl<E: std::error::Error + 'static> From<bb8::RunError<E>> for ConnectionError {
    fn from(err: bb8::RunError<E>) -> Self {
        ConnectionError::PoolError(err.to_string())
    }
}

/// Type alias for our connection pool
pub type MssqlPool = Pool<ConnectionManager>;

/// Manages multiple MS-SQL connections with pooling
pub struct MssqlConnectionManager {
    /// Map of connection ID -> connection pool
    pools: RwLock<HashMap<String, Arc<MssqlPool>>>,
    /// Map of connection ID -> connection config
    configs: RwLock<HashMap<String, ConnectionConfig>>,
}

impl MssqlConnectionManager {
    pub fn new() -> Self {
        Self {
            pools: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
        }
    }

    /// Add a new connection configuration (does not connect yet)
    pub async fn add_connection(&self, config: ConnectionConfig) -> Result<String, ConnectionError> {
        let id = config.id.clone();
        let mut configs = self.configs.write().await;
        configs.insert(id.clone(), config);
        Ok(id)
    }

    /// Test a connection without adding it to the pool
    pub async fn test_connection(&self, config: &ConnectionConfig) -> Result<bool, ConnectionError> {
        let tiberius_config = config.to_tiberius_config()?;
        
        // Try to establish a connection
        let tcp = tokio::net::TcpStream::connect(format!("{}:{}", config.host, config.port))
            .await
            .map_err(|e| ConnectionError::ConnectionFailed(format!("TCP connection failed: {}", e)))?;
        
        tcp.set_nodelay(true)
            .map_err(|e| ConnectionError::ConnectionFailed(format!("Failed to set TCP_NODELAY: {}", e)))?;
        
        let _client = tiberius::Client::connect(tiberius_config, tcp.compat_write())
            .await
            .map_err(ConnectionError::from)?;
        
        Ok(true)
    }

    /// Connect to a database (creates pool if not exists)
    pub async fn connect(&self, connection_id: &str) -> Result<Arc<MssqlPool>, ConnectionError> {
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
        let tiberius_config = config.to_tiberius_config()?;
        let manager = ConnectionManager::build(tiberius_config)
            .map_err(|e| ConnectionError::ConfigError(e.to_string()))?;
        
        let pool = Pool::builder()
            .max_size(5)
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

    /// Create a dedicated (non-pooled) connection for cancellable queries
    /// Returns the TCP stream and client separately so the stream can be dropped to cancel
    pub async fn create_dedicated_connection(&self, connection_id: &str) -> Result<tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>, ConnectionError> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id)
                .ok_or_else(|| ConnectionError::NotFound(connection_id.to_string()))?
                .clone()
        };
        
        let tiberius_config = config.to_tiberius_config()?;
        
        let tcp = tokio::net::TcpStream::connect(format!("{}:{}", config.host, config.port))
            .await
            .map_err(|e| ConnectionError::ConnectionFailed(format!("TCP connection failed: {}", e)))?;
        
        tcp.set_nodelay(true)
            .map_err(|e| ConnectionError::ConnectionFailed(format!("Failed to set TCP_NODELAY: {}", e)))?;
        
        let client = tiberius::Client::connect(tiberius_config, tcp.compat_write())
            .await
            .map_err(ConnectionError::from)?;
        
        Ok(client)
    }

    /// Disconnect a specific connection
    pub async fn disconnect(&self, connection_id: &str) -> Result<(), ConnectionError> {
        let mut pools = self.pools.write().await;
        pools.remove(connection_id);
        Ok(())
    }

    /// Remove a connection configuration
    pub async fn remove_connection(&self, connection_id: &str) -> Result<(), ConnectionError> {
        self.disconnect(connection_id).await?;
        let mut configs = self.configs.write().await;
        configs.remove(connection_id);
        Ok(())
    }

    /// Get a pool by connection ID
    pub async fn get_pool(&self, connection_id: &str) -> Option<Arc<MssqlPool>> {
        let pools = self.pools.read().await;
        pools.get(connection_id).map(Arc::clone)
    }

    /// List all configured connections
    pub async fn list_connections(&self) -> Vec<ConnectionInfo> {
        let configs = self.configs.read().await;
        let pools = self.pools.read().await;
        
        configs.values().map(|config| {
            let mut info = ConnectionInfo::from(config);
            info.is_connected = pools.contains_key(&config.id);
            info
        }).collect()
    }

    /// Get connections for a specific space
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

    /// Update a connection configuration
    pub async fn update_connection(&self, connection_id: &str, updates: ConnectionConfigUpdate) -> Result<ConnectionInfo, ConnectionError> {
        let mut configs = self.configs.write().await;
        let config = configs.get_mut(connection_id)
            .ok_or_else(|| ConnectionError::NotFound(connection_id.to_string()))?;
        
        if let Some(name) = updates.name {
            config.name = name;
        }
        if let Some(host) = updates.host {
            config.host = host;
        }
        if let Some(port) = updates.port {
            config.port = port;
        }
        if let Some(database) = updates.database {
            config.database = database;
        }
        if let Some(username) = updates.username {
            config.username = username;
        }
        if let Some(password) = updates.password {
            config.password = password;
        }
        if let Some(trust_certificate) = updates.trust_certificate {
            config.trust_certificate = trust_certificate;
        }
        if let Some(encrypt) = updates.encrypt {
            config.encrypt = encrypt;
        }
        if let Some(space_id) = updates.space_id {
            config.space_id = space_id;
        }
        
        // Disconnect if connected (config changed)
        drop(configs);
        self.disconnect(connection_id).await?;
        
        let configs = self.configs.read().await;
        let config = configs.get(connection_id).unwrap();
        Ok(ConnectionInfo::from(config))
    }

    /// Check connection health
    pub async fn is_healthy(&self, connection_id: &str) -> bool {
        if let Some(pool) = self.get_pool(connection_id).await {
            match pool.get().await {
                Ok(_conn) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Get list of accessible databases from a connection (only databases the user has access to)
    pub async fn get_databases(&self, connection_id: &str) -> Result<Vec<String>, ConnectionError> {
        let pool = self.connect(connection_id).await?;
        let mut conn = pool.get().await?;
        
        let query = "SELECT name FROM sys.databases WHERE state_desc = 'ONLINE' AND HAS_DBACCESS(name) = 1 ORDER BY name";
        let stream = conn.simple_query(query).await?;
        let rows = stream.into_first_result().await?;
        
        let databases: Vec<String> = rows
            .iter()
            .filter_map(|row| row.get::<&str, _>(0).map(|s| s.to_string()))
            .collect();
        
        Ok(databases)
    }

    /// Get list of all online databases with an access flag (name, has_access).
    /// Uses two separate queries and merges in Rust to avoid tiberius type ambiguity
    /// with HAS_DBACCESS() returning typed I32 vs text depending on the protocol path.
    pub async fn get_databases_with_access(&self, connection_id: &str) -> Result<Vec<(String, bool)>, ConnectionError> {
        let pool = self.connect(connection_id).await?;
        let mut conn = pool.get().await?;

        // Query 1: all online databases (name only, &str is unambiguous)
        let all_query = "SELECT name FROM sys.databases WHERE state_desc = 'ONLINE' ORDER BY name";
        let stream = conn.simple_query(all_query).await?;
        let all_rows = stream.into_first_result().await?;
        let all_dbs: Vec<String> = all_rows
            .iter()
            .filter_map(|row| row.get::<&str, _>(0).map(|s| s.to_string()))
            .collect();

        // Query 2: only accessible databases — reuse the exact proven query
        let access_query = "SELECT name FROM sys.databases WHERE state_desc = 'ONLINE' AND HAS_DBACCESS(name) = 1 ORDER BY name";
        let stream2 = conn.simple_query(access_query).await?;
        let access_rows = stream2.into_first_result().await?;
        let accessible: std::collections::HashSet<String> = access_rows
            .iter()
            .filter_map(|row| row.get::<&str, _>(0).map(|s| s.to_string()))
            .collect();

        // Merge: mark each database as accessible or not
        let result = all_dbs
            .into_iter()
            .map(|name| {
                let has_access = accessible.contains(&name);
                (name, has_access)
            })
            .collect();

        Ok(result)
    }

    /// Get a connection config by ID
    pub async fn get_connection(&self, connection_id: &str) -> Option<ConnectionInfo> {
        let configs = self.configs.read().await;
        let pools = self.pools.read().await;
        
        configs.get(connection_id).map(|config| {
            let mut info = ConnectionInfo::from(config);
            info.is_connected = pools.contains_key(&config.id);
            info
        })
    }
}

impl Default for MssqlConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Updates for connection configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionConfigUpdate {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub trust_certificate: Option<bool>,
    pub encrypt: Option<bool>,
    pub space_id: Option<Option<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_config_new() {
        let config = ConnectionConfig::new(
            "Test DB".to_string(),
            "localhost".to_string(),
            1433,
            "master".to_string(),
            "sa".to_string(),
            "password123".to_string(),
        );

        assert!(!config.id.is_empty());
        assert_eq!(config.name, "Test DB");
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 1433);
        assert_eq!(config.database, "master");
        assert_eq!(config.username, "sa");
        assert_eq!(config.password, "password123");
        assert_eq!(config.trust_certificate, true); // Default
        assert_eq!(config.encrypt, false); // Default
        assert!(config.space_id.is_none());
    }

    #[test]
    fn test_connection_info_from_config() {
        let config = ConnectionConfig::new(
            "Test DB".to_string(),
            "localhost".to_string(),
            1433,
            "master".to_string(),
            "sa".to_string(),
            "password123".to_string(),
        );

        let info = ConnectionInfo::from(&config);

        assert_eq!(info.id, config.id);
        assert_eq!(info.name, config.name);
        assert_eq!(info.host, config.host);
        assert_eq!(info.port, config.port);
        assert_eq!(info.database, config.database);
        assert_eq!(info.username, config.username);
        // Password should NOT be in ConnectionInfo (it's for frontend)
        // But ConnectionInfo struct doesn't even HAVE a password field, so this checks itself by compilation.
        assert_eq!(info.trust_certificate, config.trust_certificate);
        assert_eq!(info.encrypt, config.encrypt);
        assert_eq!(info.space_id, config.space_id);
        assert_eq!(info.is_connected, false);
    }

    #[test]
    fn test_tiberius_config_conversion() {
         let mut config = ConnectionConfig::new(
            "Test DB".to_string(),
            "localhost".to_string(),
            1433,
            "master".to_string(),
            "sa".to_string(),
            "password123".to_string(),
        );
        config.encrypt = true;
        config.trust_certificate = false;

        let tiberius_config = config.to_tiberius_config();
        assert!(tiberius_config.is_ok());

        // We can't easily inspect the internal state of tiberius::Config, 
        // but ensuring it doesn't error is a good first step.
    }
}
