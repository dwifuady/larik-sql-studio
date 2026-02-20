// Unified Connection Manager
// Delegates to specific connection managers based on connection ID or type

use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use crate::db::connection::{MssqlConnectionManager, ConnectionError, ConnectionInfo, ConnectionConfig};
use crate::db::postgres_manager::{PostgresConnectionManager, PostgresConfig};
use crate::db::sqlite_manager::{SqliteConnectionManager, SqliteConfig};
use crate::db::traits::{DatabaseType, DatabaseConfig};

pub struct UnifiedConnectionManager {
    mssql: Arc<MssqlConnectionManager>,
    postgres: Arc<PostgresConnectionManager>,
    sqlite: Arc<SqliteConnectionManager>,
    // Track which type each connection ID belongs to
    connection_types: RwLock<HashMap<String, DatabaseType>>,
}

impl UnifiedConnectionManager {
    pub fn new() -> Self {
        Self {
            mssql: Arc::new(MssqlConnectionManager::new()),
            postgres: Arc::new(PostgresConnectionManager::new()),
            sqlite: Arc::new(SqliteConnectionManager::new()),
            connection_types: RwLock::new(HashMap::new()),
        }
    }

    pub fn sqlite(&self) -> Arc<SqliteConnectionManager> {
        Arc::clone(&self.sqlite)
    }

    pub fn mssql(&self) -> Arc<MssqlConnectionManager> {
        Arc::clone(&self.mssql)
    }

    pub fn postgres(&self) -> Arc<PostgresConnectionManager> {
        Arc::clone(&self.postgres)
    }

    /// Register a connection type mapping
    pub async fn register_connection_type(&self, id: String, db_type: DatabaseType) {
        let mut types = self.connection_types.write().await;
        types.insert(id, db_type);
    }
    
    /// Get the database type for a connection ID
    pub async fn get_connection_type(&self, id: &str) -> Option<DatabaseType> {
        let types = self.connection_types.read().await;
        types.get(id).cloned()
    }

    /// Add a connection from a generic config
    pub async fn add_connection(&self, config: DatabaseConfig) -> Result<String, ConnectionError> {
        let id = config.id.clone();
        
        match config.database_type {
            DatabaseType::Mssql => {
                let mssql_config = ConnectionConfig {
                    id: config.id.clone(),
                    name: config.name.clone(),
                    host: config.host.clone().ok_or_else(|| ConnectionError::ConfigError("Host required".to_string()))?,
                    port: config.get_port(),
                    database: config.database.clone(),
                    username: config.username.clone().unwrap_or_default(),
                    password: config.password.clone(),
                    trust_certificate: config.mssql_trust_cert.unwrap_or(true),
                    encrypt: config.mssql_encrypt.unwrap_or(false),
                    space_id: config.space_id.clone(),
                };
                self.mssql.add_connection(mssql_config).await?;
            }
            DatabaseType::Postgresql => {
                 let pg_config = PostgresConfig {
                    id: config.id.clone(),
                    name: config.name.clone(),
                    host: config.host.clone().ok_or_else(|| ConnectionError::ConfigError("Host required".to_string()))?,
                    port: config.get_port(),
                    database: if config.database.is_empty() { "postgres".to_string() } else { config.database.clone() },
                    username: config.username.clone().unwrap_or_default(),
                    password: config.password.clone(),
                    sslmode: config.postgres_sslmode.clone().unwrap_or_else(|| "prefer".to_string()),
                    space_id: config.space_id.clone(),
                };
                self.postgres.add_connection(pg_config).await?;
            }
            DatabaseType::Sqlite => {
                // For SQLite, `config.database` is the file path
                if config.database.is_empty() {
                    return Err(ConnectionError::ConfigError("SQLite database file path is required".to_string()));
                }
                let sqlite_config = SqliteConfig {
                    id: config.id.clone(),
                    name: config.name.clone(),
                    path: config.database.clone(),
                    space_id: config.space_id.clone(),
                };
                self.sqlite.add_connection(sqlite_config).await?;
            }
            DatabaseType::Mysql => return Err(ConnectionError::ConfigError("MySQL not yet supported".to_string())),
        }
        
        self.register_connection_type(id.clone(), config.database_type).await;
        Ok(id)
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), ConnectionError> {
        let db_type = self.get_connection_type(id).await;
        match db_type {
            Some(DatabaseType::Mssql) => self.mssql.disconnect(id).await,
            Some(DatabaseType::Postgresql) => self.postgres.disconnect(id).await,
            Some(DatabaseType::Sqlite) => self.sqlite.disconnect(id).await,
            _ => Ok(()), // Ignore unknown connections
        }
    }
    
    pub async fn remove_connection(&self, id: &str) -> Result<(), ConnectionError> {
        let db_type = self.get_connection_type(id).await;
        match db_type {
            Some(DatabaseType::Mssql) => self.mssql.remove_connection(id).await?,
            Some(DatabaseType::Postgresql) => {
                self.postgres.disconnect(id).await?;
                // TODO: Remove config from PostgresManager when supported
            },
            Some(DatabaseType::Sqlite) => {
                self.sqlite.disconnect(id).await?;
            },
            _ => {},
        }
        
        let mut types = self.connection_types.write().await;
        types.remove(id);
        Ok(())
    }

    pub async fn list_connections(&self) -> Vec<ConnectionInfo> {
        let mut mssql_conns = self.mssql.list_connections().await;
        let mut pg_conns = self.postgres.list_connections().await;
        let mut sqlite_conns = self.sqlite.list_connections().await;
        mssql_conns.append(&mut pg_conns);
        mssql_conns.append(&mut sqlite_conns);
        mssql_conns
    }

    pub async fn get_connections_by_space(&self, space_id: &str) -> Vec<ConnectionInfo> {
        let mut mssql_conns = self.mssql.get_connections_by_space(space_id).await;
        let mut pg_conns = self.postgres.get_connections_by_space(space_id).await;
        let mut sqlite_conns = self.sqlite.get_connections_by_space(space_id).await;
        mssql_conns.append(&mut pg_conns);
        mssql_conns.append(&mut sqlite_conns);
        mssql_conns
    }
    
    pub async fn get_connection(&self, id: &str) -> Option<ConnectionInfo> {
        if let Some(conn) = self.mssql.get_connection(id).await {
            return Some(conn);
        }
        if let Some(conn) = self.postgres.get_connection(id).await {
            return Some(conn);
        }
        self.sqlite.get_connection(id).await
    }
    
    pub async fn update_connection(&self, id: &str, updates: crate::db::connection::ConnectionConfigUpdate) -> Result<ConnectionInfo, crate::db::connection::ConnectionError> {
        let db_type = self.get_connection_type(id).await;
        match db_type {
            Some(DatabaseType::Mssql) => self.mssql.update_connection(id, updates).await,
             Some(DatabaseType::Postgresql) => {
                 Err(crate::db::connection::ConnectionError::ConfigError("Update not implemented for PostgreSQL yet".to_string()))
             }
             _ => Err(crate::db::connection::ConnectionError::NotFound(id.to_string()))
        }
    }

    pub async fn connect(&self, id: &str) -> Result<bool, String> {
         let db_type = self.get_connection_type(id).await;
         match db_type {
            Some(DatabaseType::Mssql) => {
                self.mssql.connect(id).await.map(|_| true).map_err(|e| e.to_string())
            },
            Some(DatabaseType::Postgresql) => {
                 self.postgres.connect(id).await.map(|_| true).map_err(|e| e.to_string())
            },
            Some(DatabaseType::Sqlite) => {
                self.sqlite.connect(id).await.map(|_| true).map_err(|e| e.to_string())
            },
            _ => Err("Connection not found".to_string())
         }
    }
    
    pub async fn is_healthy(&self, id: &str) -> bool {
         let db_type = self.get_connection_type(id).await;
          match db_type {
            Some(DatabaseType::Mssql) => self.mssql.is_healthy(id).await,
            Some(DatabaseType::Postgresql) => self.postgres.is_healthy(id).await,
            Some(DatabaseType::Sqlite) => self.sqlite.is_healthy(id).await,
            _ => false
          }
    }
    
    pub async fn get_databases(&self, id: &str) -> Result<Vec<String>, String> {
        let db_type = self.get_connection_type(id).await;
        match db_type {
            Some(DatabaseType::Mssql) => self.mssql.get_databases(id).await.map_err(|e| e.to_string()),
            Some(DatabaseType::Postgresql) => {
                 Err("Get databases not implemented for Postgres manager yet".to_string())
            },
            Some(DatabaseType::Sqlite) => {
                // SQLite returns attached databases (main + any ATTACH'd)
                // For the UI, we show the file name as the single "database"
                if let Some(config) = self.sqlite.get_config(id).await {
                    let file_name = std::path::Path::new(&config.path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| config.path.clone());
                    Ok(vec![file_name])
                } else {
                    Err("Connection not found".to_string())
                }
            },
            _ => Err("Connection not found".to_string())
        }
    }

    /// Get a DatabaseConfig suitable for query execution
    pub async fn get_database_config_for(&self, id: &str) -> Option<crate::db::traits::DatabaseConfig> {
        let db_type = self.get_connection_type(id).await?;
        match db_type {
            DatabaseType::Sqlite => self.sqlite.get_database_config(id).await,
            _ => None,
        }
    }
}
