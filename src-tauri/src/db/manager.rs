// Connection registry (T015, T016)
// Holds connection configurations and routes each operation to the driver
// selected by `ConnectionConfig::driver`. Pools/clients are owned by the drivers.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::db::config::{ConnectionConfig, ConnectionConfigUpdate, ConnectionInfo};
use crate::db::driver::{DatabaseDriver, DatabaseEntry, DriverRegistry};
use crate::db::error::ConnectionError;

/// Manages connection configurations across all database drivers
pub struct ConnectionManager {
    drivers: DriverRegistry,
    /// Map of connection ID -> connection config
    configs: RwLock<HashMap<String, ConnectionConfig>>,
}

impl ConnectionManager {
    pub fn new(drivers: DriverRegistry) -> Self {
        Self {
            drivers,
            configs: RwLock::new(HashMap::new()),
        }
    }

    /// Look up a connection's config together with the driver that serves it.
    pub async fn resolve(
        &self,
        connection_id: &str,
    ) -> Result<(Arc<dyn DatabaseDriver>, ConnectionConfig), ConnectionError> {
        let config = {
            let configs = self.configs.read().await;
            configs
                .get(connection_id)
                .ok_or_else(|| ConnectionError::NotFound(connection_id.to_string()))?
                .clone()
        };
        let driver = self.drivers.get(config.driver)?;
        Ok((driver, config))
    }

    /// Add a new connection configuration (does not connect yet)
    pub async fn add_connection(
        &self,
        config: ConnectionConfig,
    ) -> Result<String, ConnectionError> {
        // Fail early if no driver is registered for this kind
        self.drivers.get(config.driver)?;
        let id = config.id.clone();
        let mut configs = self.configs.write().await;
        configs.insert(id.clone(), config);
        Ok(id)
    }

    /// Test a connection without registering it
    pub async fn test_connection(
        &self,
        config: &ConnectionConfig,
    ) -> Result<bool, ConnectionError> {
        self.drivers
            .get(config.driver)?
            .test_connection(config)
            .await?;
        Ok(true)
    }

    /// Connect to a database (creates the driver's pool if it does not exist)
    pub async fn connect(&self, connection_id: &str) -> Result<(), ConnectionError> {
        let (driver, config) = self.resolve(connection_id).await?;
        driver.connect(&config).await
    }

    /// Disconnect a specific connection.
    /// Asks every driver, so a pool is released even if the config's driver changed.
    pub async fn disconnect(&self, connection_id: &str) -> Result<(), ConnectionError> {
        for driver in self.drivers.all() {
            driver.disconnect(connection_id).await;
        }
        Ok(())
    }

    /// Remove a connection configuration
    pub async fn remove_connection(&self, connection_id: &str) -> Result<(), ConnectionError> {
        self.disconnect(connection_id).await?;
        let mut configs = self.configs.write().await;
        configs.remove(connection_id);
        Ok(())
    }

    /// Build the frontend view of a config, including live connection state
    async fn info_for(&self, config: &ConnectionConfig) -> ConnectionInfo {
        let mut info = ConnectionInfo::from(config);
        info.is_connected = match self.drivers.get(config.driver) {
            Ok(driver) => driver.is_connected(&config.id).await,
            Err(_) => false,
        };
        info
    }

    /// List all configured connections
    pub async fn list_connections(&self) -> Vec<ConnectionInfo> {
        let configs: Vec<ConnectionConfig> = self.configs.read().await.values().cloned().collect();
        let mut infos = Vec::with_capacity(configs.len());
        for config in &configs {
            infos.push(self.info_for(config).await);
        }
        infos
    }

    /// Get connections for a specific space
    pub async fn get_connections_by_space(&self, space_id: &str) -> Vec<ConnectionInfo> {
        let configs: Vec<ConnectionConfig> = self
            .configs
            .read()
            .await
            .values()
            .filter(|config| config.space_id.as_deref() == Some(space_id))
            .cloned()
            .collect();
        let mut infos = Vec::with_capacity(configs.len());
        for config in &configs {
            infos.push(self.info_for(config).await);
        }
        infos
    }

    /// Get a connection config by ID
    pub async fn get_connection(&self, connection_id: &str) -> Option<ConnectionInfo> {
        let config = self.configs.read().await.get(connection_id).cloned()?;
        Some(self.info_for(&config).await)
    }

    /// Update a connection configuration
    pub async fn update_connection(
        &self,
        connection_id: &str,
        updates: ConnectionConfigUpdate,
    ) -> Result<ConnectionInfo, ConnectionError> {
        if let Some(driver) = updates.driver {
            self.drivers.get(driver)?;
        }

        let config = {
            let mut configs = self.configs.write().await;
            let config = configs
                .get_mut(connection_id)
                .ok_or_else(|| ConnectionError::NotFound(connection_id.to_string()))?;
            updates.apply_to(config);
            config.clone()
        };

        // Disconnect if connected (config changed)
        self.disconnect(connection_id).await?;

        Ok(ConnectionInfo::from(&config))
    }

    /// Check connection health
    pub async fn is_healthy(&self, connection_id: &str) -> bool {
        match self.resolve(connection_id).await {
            Ok((driver, _)) => driver.is_healthy(connection_id).await,
            Err(_) => false,
        }
    }

    /// Get list of accessible databases from a connection (only databases the user has access to)
    pub async fn get_databases(&self, connection_id: &str) -> Result<Vec<String>, ConnectionError> {
        Ok(self
            .get_databases_with_access(connection_id)
            .await?
            .into_iter()
            .filter(|db| db.has_access)
            .map(|db| db.name)
            .collect())
    }

    /// Get list of all online databases with an access flag
    pub async fn get_databases_with_access(
        &self,
        connection_id: &str,
    ) -> Result<Vec<DatabaseEntry>, ConnectionError> {
        let (driver, config) = self.resolve(connection_id).await?;
        driver.list_databases(&config).await
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new(DriverRegistry::with_builtin_drivers())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::driver::DriverKind;

    fn config(id: &str, space_id: Option<&str>) -> ConnectionConfig {
        let mut c = ConnectionConfig::new(
            DriverKind::Mssql,
            "Test".to_string(),
            "localhost".to_string(),
            1433,
            "master".to_string(),
            "sa".to_string(),
            "secret".to_string(),
        );
        c.id = id.to_string();
        c.space_id = space_id.map(str::to_string);
        c
    }

    #[tokio::test]
    async fn add_requires_a_registered_driver() {
        let manager = ConnectionManager::new(DriverRegistry::empty());
        let err = manager.add_connection(config("a", None)).await.unwrap_err();
        assert!(matches!(err, ConnectionError::UnsupportedDriver(_)));
        assert!(manager.list_connections().await.is_empty());
    }

    #[tokio::test]
    async fn tracks_configs_without_connecting() {
        let manager = ConnectionManager::default();
        manager
            .add_connection(config("a", Some("s1")))
            .await
            .unwrap();
        manager
            .add_connection(config("b", Some("s2")))
            .await
            .unwrap();

        assert_eq!(manager.list_connections().await.len(), 2);
        let by_space = manager.get_connections_by_space("s1").await;
        assert_eq!(by_space.len(), 1);
        assert_eq!(by_space[0].id, "a");
        assert!(!by_space[0].is_connected);
        assert!(!manager.is_healthy("a").await);

        manager.remove_connection("a").await.unwrap();
        assert!(manager.get_connection("a").await.is_none());
    }

    #[tokio::test]
    async fn update_changes_fields_and_reports_missing() {
        let manager = ConnectionManager::default();
        manager.add_connection(config("a", None)).await.unwrap();

        let info = manager
            .update_connection(
                "a",
                ConnectionConfigUpdate {
                    driver: None,
                    name: Some("Renamed".to_string()),
                    host: None,
                    port: None,
                    database: None,
                    username: None,
                    password: None,
                    trust_certificate: None,
                    encrypt: None,
                    space_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(info.name, "Renamed");

        let missing = manager
            .update_connection(
                "nope",
                ConnectionConfigUpdate {
                    driver: None,
                    name: None,
                    host: None,
                    port: None,
                    database: None,
                    username: None,
                    password: None,
                    trust_certificate: None,
                    encrypt: None,
                    space_id: None,
                },
            )
            .await;
        assert!(matches!(missing, Err(ConnectionError::NotFound(_))));
    }
}
