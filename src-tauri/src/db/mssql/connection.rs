// MS-SQL Connection Management (T015, T016)
// tiberius client config, error mapping, pooling and dedicated connections

use bb8::Pool;
use secrecy::ExposeSecret;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tiberius::{AuthMethod, Config, EncryptionLevel};
use tokio::sync::RwLock;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use crate::db::config::ConnectionConfig;
use crate::db::error::ConnectionError;

/// Type alias for our connection pool
pub type MssqlPool = Pool<MssqlConnectionManager>;

/// bb8 pool manager for tiberius clients.
///
/// Written in-house instead of using the bb8-tiberius crate: its latest
/// release pins tiberius 0.12, whose TLS stack (rustls 0.21 / webpki 0.101)
/// has no patched release for RUSTSEC-2026-0382. This keeps pooling on the
/// patched tiberius 0.13 / tokio-rustls 0.26 stack.
#[derive(Clone)]
pub struct MssqlConnectionManager {
    config: Config,
}

impl bb8::ManageConnection for MssqlConnectionManager {
    type Connection = MssqlClient;
    type Error = crate::db::mssql::TiberiusError;

    #[allow(refining_impl_trait)]
    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let tcp = tokio::net::TcpStream::connect(&self.config.get_addr())
            .await
            .map_err(crate::db::mssql::TiberiusError::Io)?;
        tcp.set_nodelay(true)
            .map_err(|e| crate::db::mssql::TiberiusError::Io(std::io::Error::other(e)))?;
        tiberius::Client::connect(self.config.clone(), tcp.compat_write())
            .await
            .map_err(crate::db::mssql::TiberiusError::Tiberius)
    }

    #[allow(refining_impl_trait)]
    async fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        let mut stream = conn
            .simple_query("SELECT 1")
            .await
            .map_err(crate::db::mssql::TiberiusError::Tiberius)?;
        // Drain the stream so the ping completes server-side
        use futures::StreamExt;
        while let Some(item) = stream.next().await {
            item.map_err(crate::db::mssql::TiberiusError::Tiberius)?;
        }
        Ok(())
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

/// A single (non-pooled) tiberius client over TCP
pub type MssqlClient = tiberius::Client<Compat<tokio::net::TcpStream>>;

impl From<tiberius::error::Error> for ConnectionError {
    fn from(err: tiberius::error::Error) -> Self {
        // Check for password expired / must-change errors from SQL Server
        // 18487: The password has expired (and the account is locked)
        // 18488: The password must be changed before first use
        // 18456 with state 2: Login failed — password change required
        // 18456 with state 1: Login failed — password expired
        if let tiberius::error::Error::Server(e) = &err {
            match e.code() {
                18487 | 18488 => return ConnectionError::PasswordExpired,
                18456 => {
                    // Check state: 1 = password expired, 2 = must change
                    if e.state() == 1 || e.state() == 2 {
                        return ConnectionError::PasswordExpired;
                    }
                }
                _ => {}
            }
        }
        ConnectionError::ConnectionFailed(err.to_string())
    }
}

/// Create a tiberius Config from a ConnectionConfig
pub fn tiberius_config(config: &ConnectionConfig) -> Result<Config, ConnectionError> {
    let mut tiberius_config = Config::new();
    tiberius_config.host(&config.host);
    tiberius_config.port(config.port);
    tiberius_config.database(&config.database);
    tiberius_config.authentication(AuthMethod::sql_server(
        &config.username,
        config.password.expose_secret(),
    ));

    if config.trust_certificate {
        tiberius_config.trust_cert();
    }

    tiberius_config.encryption(if config.encrypt {
        EncryptionLevel::Required
    } else {
        EncryptionLevel::Off
    });

    Ok(tiberius_config)
}

/// Create a dedicated (non-pooled) connection.
/// Used for cancellable queries (dropping the client closes TCP, which cancels
/// the query server-side) and for anything that issues `USE`, so pooled
/// connections never have their database context changed.
pub async fn connect_dedicated(config: &ConnectionConfig) -> Result<MssqlClient, ConnectionError> {
    let tiberius_config = tiberius_config(config)?;

    let tcp = tokio::net::TcpStream::connect(format!("{}:{}", config.host, config.port))
        .await
        .map_err(|e| ConnectionError::ConnectionFailed(format!("TCP connection failed: {}", e)))?;

    tcp.set_nodelay(true).map_err(|e| {
        ConnectionError::ConnectionFailed(format!("Failed to set TCP_NODELAY: {}", e))
    })?;

    let client = tiberius::Client::connect(tiberius_config, tcp.compat_write())
        .await
        .map_err(ConnectionError::from)?;

    Ok(client)
}

/// Pools keyed by connection ID
#[derive(Default)]
pub struct MssqlPools {
    pools: RwLock<HashMap<String, Arc<MssqlPool>>>,
}

impl MssqlPools {
    /// Get the pool for `config.id`, creating it if it does not exist
    pub async fn get_or_create(
        &self,
        config: &ConnectionConfig,
    ) -> Result<Arc<MssqlPool>, ConnectionError> {
        if let Some(pool) = self.get(&config.id).await {
            return Ok(pool);
        }

        let manager = MssqlConnectionManager {
            config: tiberius_config(config)?,
        };

        let pool = Pool::builder()
            .connection_timeout(Duration::from_secs(5))
            .max_lifetime(Some(Duration::from_secs(300)))
            .min_idle(Some(1))
            .max_size(5)
            .build(manager)
            .await
            .map_err(|e| ConnectionError::PoolError(e.to_string()))?;

        let pool = Arc::new(pool);
        let mut pools = self.pools.write().await;
        // Another task may have raced us; keep whichever landed first
        Ok(Arc::clone(
            pools.entry(config.id.clone()).or_insert_with(|| pool),
        ))
    }

    pub async fn get(&self, connection_id: &str) -> Option<Arc<MssqlPool>> {
        self.pools.read().await.get(connection_id).map(Arc::clone)
    }

    pub async fn remove(&self, connection_id: &str) {
        self.pools.write().await.remove(connection_id);
    }

    pub async fn contains(&self, connection_id: &str) -> bool {
        self.pools.read().await.contains_key(connection_id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::driver::DriverKind;

    #[test]
    fn test_tiberius_config_conversion() {
        let mut config = ConnectionConfig::new(
            DriverKind::Mssql,
            "Test DB".to_string(),
            "localhost".to_string(),
            1433,
            "master".to_string(),
            "sa".to_string(),
            "password123".to_string(),
        );
        config.encrypt = true;
        config.trust_certificate = false;

        let tiberius_config = tiberius_config(&config);
        assert!(tiberius_config.is_ok());

        // We can't easily inspect the internal state of tiberius::Config,
        // but ensuring it doesn't error is a good first step.
    }
}
