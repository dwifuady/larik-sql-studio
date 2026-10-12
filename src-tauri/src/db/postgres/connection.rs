// PostgreSQL Connection Management
// tokio-postgres config translation, rustls TLS setup, error mapping and a
// registry of long-lived clients keyed by connection ID (mirrors the shape of
// `db/mssql/connection.rs`; the client map matches `db/mongodb/connection.rs`).

use rustls::pki_types::CertificateDer;
use secrecy::ExposeSecret;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_postgres::NoTls;
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::db::config::ConnectionConfig;
use crate::db::error::ConnectionError;

/// How long a connection attempt may take before failing.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// TLS posture derived from the neutral connection flags, matching the
/// SQL Server driver semantics: `encrypt` on = verified TLS,
/// `trust_certificate` = accept any certificate, off = plaintext.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    Disable,
    Verify,
    TrustAny,
}

impl TlsMode {
    pub fn from_config(config: &ConnectionConfig) -> Self {
        if !config.encrypt {
            Self::Disable
        } else if config.trust_certificate {
            Self::TrustAny
        } else {
            Self::Verify
        }
    }
}

/// Build a rustls client config for the mode. `None` means TLS disabled.
fn rustls_config(mode: TlsMode) -> Option<rustls::ClientConfig> {
    let provider = rustls::crypto::ring::default_provider();
    let builder = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .ok()?;
    Some(match mode {
        TlsMode::Disable => return None,
        TlsMode::Verify => {
            let mut roots = rustls::RootCertStore::empty();
            let certs = rustls_native_certs::load_native_certs();
            for cert in certs.certs {
                let _ = roots.add(cert);
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        TlsMode::TrustAny => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(TrustAnyVerifier))
            .with_no_client_auth(),
    })
}

/// Verifier that accepts every server certificate (the "Trust Certificate"
/// escape hatch). Signature algorithms still come from the ring provider.
#[derive(Debug)]
struct TrustAnyVerifier;

impl rustls::client::danger::ServerCertVerifier for TrustAnyVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Create a tokio-postgres Config from the neutral connection config.
/// `database` overrides the target database (per-tab database selection);
/// `None` keeps the configured database.
pub fn pg_config(
    config: &ConnectionConfig,
    database: Option<&str>,
) -> Result<tokio_postgres::Config, ConnectionError> {
    let database = match database {
        Some(db) if !db.is_empty() => db,
        _ if config.database.is_empty() => {
            return Err(ConnectionError::ConfigError(
                "PostgreSQL connections require a database".to_string(),
            ));
        }
        _ => config.database.as_str(),
    };

    let mut pg = tokio_postgres::Config::new();
    pg.host(config.host.as_str())
        .port(config.port)
        .dbname(database)
        .user(config.username.as_str())
        .password(config.password.expose_secret());

    Ok(pg)
}

/// Open a dedicated (non-pooled) connection. Used for cancellable queries —
/// dropping the client closes the TCP stream, which cancels the running
/// statement server-side — and for anything that must not disturb the
/// long-lived client.
pub async fn connect_dedicated(
    config: &ConnectionConfig,
    database: Option<&str>,
) -> Result<tokio_postgres::Client, ConnectionError> {
    let pg = pg_config(config, database)?;
    let mode = TlsMode::from_config(config);
    connect_with(&pg, mode).await
}

pub async fn connect_with(
    pg: &tokio_postgres::Config,
    mode: TlsMode,
) -> Result<tokio_postgres::Client, ConnectionError> {
    let attempt = async {
        match rustls_config(mode) {
            None => {
                let (client, connection) = pg.connect(NoTls).await?;
                tokio::spawn(connection);
                Ok(client)
            }
            Some(cfg) => {
                let (client, connection) = pg.connect(MakeRustlsConnect::new(cfg)).await?;
                tokio::spawn(connection);
                Ok(client)
            }
        }
    };
    let client = tokio::time::timeout(CONNECT_TIMEOUT, attempt)
        .await
        .map_err(|_| ConnectionError::Timeout)?
        .map_err(connect_error)?;
    Ok(client)
}

/// Map a failure while establishing a connection.
pub fn connect_error(err: tokio_postgres::Error) -> ConnectionError {
    ConnectionError::ConnectionFailed(describe_error(&err))
}

/// Map an error raised while running a statement.
/// A closed/reset connection means the query was cancelled underneath us.
pub fn statement_error(err: tokio_postgres::Error) -> ConnectionError {
    let msg = describe_error(&err);
    if msg.contains("connection closed") || msg.contains("Connection closed") {
        ConnectionError::Cancelled
    } else {
        ConnectionError::QueryError(msg)
    }
}

/// Full user-facing message: tokio-postgres `Display` only says "db error";
/// the server-side detail lives on the `DbError`.
fn describe_error(err: &tokio_postgres::Error) -> String {
    match err.as_db_error() {
        Some(db) => {
            let mut msg = db.message().to_string();
            if let Some(detail) = db.detail() {
                msg.push_str(&format!(" — {detail}"));
            }
            msg
        }
        None => err.to_string(),
    }
}

/// Long-lived clients keyed by connection ID.
#[derive(Default)]
pub struct PgClients {
    clients: RwLock<HashMap<String, Arc<tokio_postgres::Client>>>,
}

impl PgClients {
    /// Get the client for `config.id`, creating it if it does not exist.
    /// A stored client that fails its health check is replaced.
    pub async fn get_or_create(
        &self,
        config: &ConnectionConfig,
    ) -> Result<Arc<tokio_postgres::Client>, ConnectionError> {
        if let Some(client) = self.get(&config.id).await {
            if is_healthy(&client).await {
                return Ok(client);
            }
            self.remove(&config.id).await;
        }

        let client = Arc::new(connect_dedicated(config, None).await?);
        let mut clients = self.clients.write().await;
        // Another task may have raced us; keep whichever landed first.
        Ok(Arc::clone(
            clients.entry(config.id.clone()).or_insert(client),
        ))
    }

    pub async fn get(&self, connection_id: &str) -> Option<Arc<tokio_postgres::Client>> {
        self.clients.read().await.get(connection_id).map(Arc::clone)
    }

    pub async fn remove(&self, connection_id: &str) {
        self.clients.write().await.remove(connection_id);
    }

    pub async fn contains(&self, connection_id: &str) -> bool {
        self.clients.read().await.contains_key(connection_id)
    }
}

/// Whether the client responds to a cheap round-trip.
pub async fn is_healthy(client: &tokio_postgres::Client) -> bool {
    tokio::time::timeout(Duration::from_secs(3), client.simple_query("SELECT 1"))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn config(encrypt: bool, trust: bool) -> ConnectionConfig {
        let mut c = ConnectionConfig::new(
            crate::db::driver::DriverKind::Postgres,
            "pg".to_string(),
            "localhost".to_string(),
            5432,
            "postgres".to_string(),
            "postgres".to_string(),
            "secret".to_string(),
        );
        c.encrypt = encrypt;
        c.trust_certificate = trust;
        c
    }


    #[test]
    fn tls_mode_matches_encrypt_flags() {
        assert_eq!(TlsMode::from_config(&config(false, false)), TlsMode::Disable);
        assert_eq!(TlsMode::from_config(&config(true, false)), TlsMode::Verify);
        assert_eq!(TlsMode::from_config(&config(true, true)), TlsMode::TrustAny);
    }

    #[test]
    fn builds_config_with_overrides() {
        let cfg = config(false, false);
        let pg = pg_config(&cfg, Some("other_db")).unwrap();
        assert_eq!(pg.get_dbname(), Some("other_db"));
        let pg = pg_config(&cfg, None).unwrap();
        assert_eq!(pg.get_dbname(), Some("postgres"));
        assert_eq!(pg.get_user(), Some("postgres"));
        assert_eq!(pg.get_ports(), &[5432]);
    }

    #[test]
    fn empty_database_is_a_config_error() {
        let mut cfg = config(false, false);
        cfg.database = String::new();
        assert!(matches!(
            pg_config(&cfg, None),
            Err(ConnectionError::ConfigError(_))
        ));
    }
}
