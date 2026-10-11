// MongoDB connection management
// Builds client options from a ConnectionConfig and caches one Client per
// connection ID (the driver's Client is itself a connection pool).

use mongodb::bson::doc;
use mongodb::error::ErrorKind;
use mongodb::options::{ClientOptions, Credential, ServerAddress, Tls, TlsOptions};
use mongodb::Client;
use secrecy::ExposeSecret;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::db::config::ConnectionConfig;
use crate::db::error::ConnectionError;

const TIMEOUT: Duration = Duration::from_secs(5);

/// Whether `host` holds a full connection string rather than a hostname
pub fn is_connection_string(host: &str) -> bool {
    let host = host.trim_start();
    host.starts_with("mongodb://") || host.starts_with("mongodb+srv://")
}

impl From<mongodb::error::Error> for ConnectionError {
    fn from(err: mongodb::error::Error) -> Self {
        match err.kind.as_ref() {
            ErrorKind::Authentication { message, .. } => {
                ConnectionError::ConnectionFailed(format!("Authentication failed: {message}"))
            }
            ErrorKind::ServerSelection { .. }
            | ErrorKind::Io(_)
            | ErrorKind::DnsResolve { .. }
            | ErrorKind::InvalidTlsConfig { .. }
            | ErrorKind::ConnectionPoolCleared { .. } => {
                ConnectionError::ConnectionFailed(err.to_string())
            }
            // Command/write/argument errors are about the statement itself
            _ => ConnectionError::QueryError(err.to_string()),
        }
    }
}

/// Build driver options.
///
/// - `host` may be a full `mongodb://` / `mongodb+srv://` string (e.g. Atlas). The string
///   is authoritative: port, Trust Certificate and Encrypt are ignored; a username and
///   password entered separately replace any credentials in the string.
/// - Otherwise host/port are used directly, Encrypt enables TLS and Trust Certificate
///   allows invalid certificates.
pub async fn client_options(config: &ConnectionConfig) -> Result<ClientOptions, ConnectionError> {
    let host = config.host.trim();
    let mut options = if is_connection_string(host) {
        ClientOptions::parse(host)
            .await
            .map_err(|e| ConnectionError::ConfigError(format!("Invalid connection string: {e}")))?
    } else {
        let mut options = ClientOptions::default();
        options.hosts = vec![ServerAddress::Tcp {
            host: host.to_string(),
            port: Some(config.port),
        }];
        options.tls = Some(if config.encrypt {
            let mut tls = TlsOptions::default();
            tls.allow_invalid_certificates = Some(config.trust_certificate);
            Tls::Enabled(tls)
        } else {
            Tls::Disabled
        });
        options
    };

    if !config.username.is_empty() {
        let source = options.credential.as_ref().and_then(|c| c.source.clone());
        let mut credential = Credential::default();
        credential.username = Some(config.username.clone());
        credential.password = Some(config.password.expose_secret().to_string());
        credential.source = source;
        options.credential = Some(credential);
    }

    // Like mongosh and Compass: a single host without a replica set name is a
    // direct connection, so a replica-set member that advertises internal
    // hostnames (e.g. Docker) is still reachable by the address the user typed.
    if options.direct_connection.is_none()
        && options.hosts.len() == 1
        && options.repl_set_name.is_none()
        && !host.starts_with("mongodb+srv://")
    {
        options.direct_connection = Some(true);
    }

    options.app_name = Some("Larik SQL Studio".to_string());
    options.server_selection_timeout.get_or_insert(TIMEOUT);
    options.connect_timeout.get_or_insert(TIMEOUT);
    Ok(options)
}

/// The database used when a tab has none selected
pub fn default_database(config: &ConnectionConfig, options: Option<&ClientOptions>) -> String {
    if !config.database.trim().is_empty() {
        return config.database.trim().to_string();
    }
    options
        .and_then(|o| o.default_database.clone())
        .unwrap_or_else(|| "test".to_string())
}

/// Create a client and verify the server responds
pub async fn connect_client(config: &ConnectionConfig) -> Result<Client, ConnectionError> {
    let client = Client::with_options(client_options(config).await?)?;
    ping(&client).await?;
    Ok(client)
}

pub async fn ping(client: &Client) -> Result<(), ConnectionError> {
    client
        .database("admin")
        .run_command(doc! { "ping": 1 })
        .await
        // A failed ping is always a connection-level problem
        .map_err(|e| match ConnectionError::from(e) {
            ConnectionError::QueryError(msg) => ConnectionError::ConnectionFailed(msg),
            other => other,
        })?;
    Ok(())
}

/// One cached Client per connection ID
#[derive(Default)]
pub struct MongoClients {
    clients: RwLock<HashMap<String, Client>>,
}

impl MongoClients {
    /// Get the client for `config.id`, creating (and pinging) it if needed
    pub async fn get_or_create(
        &self,
        config: &ConnectionConfig,
    ) -> Result<Client, ConnectionError> {
        if let Some(client) = self.get(&config.id).await {
            return Ok(client);
        }
        let client = connect_client(config).await?;
        let mut clients = self.clients.write().await;
        Ok(clients.entry(config.id.clone()).or_insert(client).clone())
    }

    pub async fn get(&self, connection_id: &str) -> Option<Client> {
        self.clients.read().await.get(connection_id).cloned()
    }

    pub async fn remove(&self, connection_id: &str) {
        let removed = self.clients.write().await.remove(connection_id);
        if let Some(client) = removed {
            // Close pooled sockets in the background; dropping alone also works
            tokio::spawn(async move { client.shutdown().await });
        }
    }

    pub async fn contains(&self, connection_id: &str) -> bool {
        self.clients.read().await.contains_key(connection_id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::driver::DriverKind;

    fn config(host: &str) -> ConnectionConfig {
        ConnectionConfig::new(
            DriverKind::Mongodb,
            "m".to_string(),
            host.to_string(),
            27017,
            String::new(),
            "app".to_string(),
            "secret".to_string(),
        )
    }

    #[test]
    fn detects_connection_strings() {
        assert!(is_connection_string("mongodb://localhost:27017"));
        assert!(is_connection_string("  mongodb+srv://cluster0.example.net"));
        assert!(!is_connection_string("localhost"));
    }

    #[tokio::test]
    async fn host_mode_uses_port_tls_and_credentials() {
        let mut cfg = config("db.local");
        cfg.encrypt = true;
        cfg.trust_certificate = false;
        let options = client_options(&cfg).await.unwrap();
        assert_eq!(
            options.hosts,
            vec![ServerAddress::Tcp {
                host: "db.local".to_string(),
                port: Some(27017)
            }]
        );
        assert!(matches!(
            options.tls,
            Some(Tls::Enabled(ref t)) if t.allow_invalid_certificates == Some(false)
        ));
        let credential = options.credential.unwrap();
        assert_eq!(credential.username.as_deref(), Some("app"));
        assert_eq!(options.server_selection_timeout, Some(TIMEOUT));
    }

    #[tokio::test]
    async fn connection_string_keeps_its_settings_and_auth_source() {
        let cfg =
            config("mongodb://other:pw@h1:27018,h2:27019/shop?authSource=admin&replicaSet=rs0");
        let options = client_options(&cfg).await.unwrap();
        assert_eq!(options.hosts.len(), 2);
        assert_eq!(options.repl_set_name.as_deref(), Some("rs0"));
        let credential = options.credential.as_ref().unwrap();
        // Separately entered credentials win, the string's authSource is kept
        assert_eq!(credential.username.as_deref(), Some("app"));
        assert_eq!(credential.source.as_deref(), Some("admin"));
        assert_eq!(default_database(&cfg, Some(&options)), "shop");
    }

    #[tokio::test]
    async fn single_host_connects_directly_unless_a_replica_set_is_named() {
        for host in ["192.168.0.251", "mongodb://192.168.0.251:27017/"] {
            let options = client_options(&config(host)).await.unwrap();
            assert_eq!(options.direct_connection, Some(true), "{host}");
        }
        let rs = client_options(&config("mongodb://h1:27017/?replicaSet=rs0"))
            .await
            .unwrap();
        assert_eq!(rs.direct_connection, None);
        let explicit = client_options(&config("mongodb://h1:27017/?directConnection=false"))
            .await
            .unwrap();
        assert_eq!(explicit.direct_connection, Some(false));
    }

    #[tokio::test]
    async fn invalid_connection_string_is_a_config_error() {
        let err = client_options(&config("mongodb://")).await.unwrap_err();
        assert!(matches!(err, ConnectionError::ConfigError(_)));
    }

    #[test]
    fn default_database_falls_back_to_test() {
        assert_eq!(default_database(&config("h"), None), "test");
        let mut cfg = config("h");
        cfg.database = "inventory".to_string();
        assert_eq!(default_database(&cfg, None), "inventory");
    }
}
