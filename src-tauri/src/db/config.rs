// Driver-neutral connection configuration.
// Each driver translates this into its own client config (see e.g. `mssql::connection`).

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::db::driver::DriverKind;

/// Connection configuration for any supported database driver
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    #[serde(default)]
    pub driver: DriverKind,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(skip_serializing)] // Don't serialize password; redacted via SecretString Debug
    pub password: SecretString,
    pub trust_certificate: bool,
    pub encrypt: bool,
    pub space_id: Option<String>,
}

impl ConnectionConfig {
    pub fn new(
        driver: DriverKind,
        name: String,
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            driver,
            name,
            host,
            port,
            database,
            username,
            password: SecretString::new(password.into()),
            trust_certificate: false,
            encrypt: true,
            space_id: None,
        }
    }
}

/// Serializable version of ConnectionConfig (without password for frontend)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub driver: DriverKind,
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
            driver: config.driver,
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

/// Updates for connection configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionConfigUpdate {
    pub driver: Option<DriverKind>,
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

impl ConnectionConfigUpdate {
    /// Apply every field that is `Some` to `config`.
    pub fn apply_to(self, config: &mut ConnectionConfig) {
        if let Some(driver) = self.driver {
            config.driver = driver;
        }
        if let Some(name) = self.name {
            config.name = name;
        }
        if let Some(host) = self.host {
            config.host = host;
        }
        if let Some(port) = self.port {
            config.port = port;
        }
        if let Some(database) = self.database {
            config.database = database;
        }
        if let Some(username) = self.username {
            config.username = username;
        }
        if let Some(password) = self.password {
            config.password = SecretString::new(password.into());
        }
        if let Some(trust_certificate) = self.trust_certificate {
            config.trust_certificate = trust_certificate;
        }
        if let Some(encrypt) = self.encrypt {
            config.encrypt = encrypt;
        }
        if let Some(space_id) = self.space_id {
            config.space_id = space_id;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn sample_config() -> ConnectionConfig {
        ConnectionConfig::new(
            DriverKind::Mssql,
            "Test DB".to_string(),
            "localhost".to_string(),
            1433,
            "master".to_string(),
            "sa".to_string(),
            "password123".to_string(),
        )
    }

    #[test]
    fn test_connection_config_new() {
        let config = sample_config();

        assert!(!config.id.is_empty());
        assert_eq!(config.driver, DriverKind::Mssql);
        assert_eq!(config.name, "Test DB");
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 1433);
        assert_eq!(config.database, "master");
        assert_eq!(config.username, "sa");
        assert_eq!(config.password.expose_secret(), "password123");
        assert_eq!(config.trust_certificate, false); // Default now false for security
        assert_eq!(config.encrypt, true); // Default now true
        assert!(config.space_id.is_none());
        // Debug must be redacted — SecretString should not leak the password
        let dbg = format!("{:?}", config);
        assert!(!dbg.contains("password123"));
    }

    #[test]
    fn test_connection_info_from_config() {
        let config = sample_config();

        let info = ConnectionInfo::from(&config);

        assert_eq!(info.id, config.id);
        assert_eq!(info.driver, config.driver);
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
    fn test_update_applies_only_present_fields() {
        let mut config = sample_config();
        ConnectionConfigUpdate {
            driver: None,
            name: Some("Renamed".to_string()),
            host: None,
            port: Some(14330),
            database: None,
            username: None,
            password: Some("new-secret".to_string()),
            trust_certificate: None,
            encrypt: None,
            space_id: Some(Some("space-1".to_string())),
        }
        .apply_to(&mut config);

        assert_eq!(config.name, "Renamed");
        assert_eq!(config.port, 14330);
        assert_eq!(config.host, "localhost");
        assert_eq!(config.password.expose_secret(), "new-secret");
        assert_eq!(config.space_id.as_deref(), Some("space-1"));
    }
}
