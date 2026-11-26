use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use futures_util::future::poll_fn;
use larik_data::read_only_sql::{PostgresReadOnlyPolicy, ReadOnlyStatementPolicy};
use larik_data::sql_language::SqlLanguage;
use larik_data::{
    ColumnSchema, ConnectionProfile, ConnectionState, DatabaseConnection, DatabaseInfo,
    DatabaseProvider, EditorLanguage, ForeignKeySchema, ProviderSetting, ProviderSettingKind,
    QueryColumn, QueryExecutionOptions, QueryResult, RoutineKind, RoutineParameter, RoutineSchema,
    TableKind, TableSchema,
};
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use tokio::task::JoinHandle;
use tokio_postgres::{AsyncMessage, Client, Config, Error as PgError, NoTls, SimpleQueryMessage};

const DEFAULT_PORT: u16 = 5432;
const DEFAULT_DATABASE: &str = "postgres";
const DEFAULT_USER: &str = "postgres";
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 8;
const DEFAULT_QUERY_TIMEOUT_SECS: u64 = 60;

const UNVERIFIED_TLS_WARNING: &str =
    "This connection does not verify the server's certificate, so someone on the network could impersonate the server.";

const SETTINGS: &[ProviderSetting] = &[
    ProviderSetting {
        key: "host",
        label: "Host",
        kind: ProviderSettingKind::Text,
        required: true,
        default_value: Some("localhost"),
        placeholder: Some("localhost or a host name"),
        hint: None,
        is_advanced: false,
    },
    ProviderSetting {
        key: "port",
        label: "Port",
        kind: ProviderSettingKind::Number,
        required: false,
        default_value: Some("5432"),
        placeholder: None,
        hint: None,
        is_advanced: true,
    },
    ProviderSetting {
        key: "database",
        label: "Database",
        kind: ProviderSettingKind::Text,
        required: false,
        default_value: Some("postgres"),
        placeholder: Some("postgres"),
        hint: None,
        is_advanced: false,
    },
    ProviderSetting {
        key: "user",
        label: "Username",
        kind: ProviderSettingKind::Text,
        required: false,
        default_value: Some("postgres"),
        placeholder: Some("postgres"),
        hint: None,
        is_advanced: false,
    },
    ProviderSetting {
        key: "password",
        label: "Password",
        kind: ProviderSettingKind::Password,
        required: false,
        default_value: None,
        placeholder: None,
        hint: None,
        is_advanced: false,
    },
    ProviderSetting {
        key: "sslMode",
        label: "SSL Mode",
        kind: ProviderSettingKind::Text,
        required: false,
        default_value: None,
        placeholder: Some("VerifyFull for remote, Prefer for localhost"),
        hint: Some(
            "Disable, Allow, Prefer, Require, VerifyCA or VerifyFull. Empty verifies remote servers (VerifyFull) and uses Prefer for localhost.",
        ),
        is_advanced: true,
    },
    ProviderSetting {
        key: "connectTimeout",
        label: "Connect timeout (seconds)",
        kind: ProviderSettingKind::Number,
        required: false,
        default_value: Some("8"),
        placeholder: None,
        hint: None,
        is_advanced: true,
    },
    ProviderSetting {
        key: "queryTimeout",
        label: "Query timeout (seconds)",
        kind: ProviderSettingKind::Number,
        required: false,
        default_value: Some("60"),
        placeholder: None,
        hint: Some(
            "How long a statement may run before the client cancels it. 0 waits indefinitely.",
        ),
        is_advanced: true,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostgresSslMode {
    Disable,
    Allow,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl PostgresSslMode {
    pub fn parse(value: Option<&str>, is_local_host: bool) -> Result<Self> {
        let Some(raw) = value.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(if is_local_host {
                Self::Prefer
            } else {
                Self::VerifyFull
            });
        };

        if raw.eq_ignore_ascii_case("disable") {
            Ok(Self::Disable)
        } else if raw.eq_ignore_ascii_case("allow") {
            Ok(Self::Allow)
        } else if raw.eq_ignore_ascii_case("prefer") {
            Ok(Self::Prefer)
        } else if raw.eq_ignore_ascii_case("require") {
            Ok(Self::Require)
        } else if raw.eq_ignore_ascii_case("verifyca") || raw.eq_ignore_ascii_case("verify-ca") {
            Ok(Self::VerifyCa)
        } else if raw.eq_ignore_ascii_case("verifyfull")
            || raw.eq_ignore_ascii_case("verify-full")
        {
            Ok(Self::VerifyFull)
        } else {
            bail!(
                "Unknown SSL mode '{raw}'. Use Disable, Allow, Prefer, Require, VerifyCA or VerifyFull."
            );
        }
    }
}

pub fn is_local_host(host: Option<&str>) -> bool {
    let Some(raw) = host.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let name = if raw.len() > 1 && raw.starts_with('[') && raw.ends_with(']') {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };
    name.eq_ignore_ascii_case("localhost")
        || name == "127.0.0.1"
        || name == "::1"
        || name.starts_with('/')
        || name.contains('\\')
}

pub fn parse_host_and_port(profile: &ConnectionProfile) -> (String, u16) {
    let fallback_port = profile.get_u16("port").unwrap_or(DEFAULT_PORT);
    let raw = profile
        .get_non_empty("host")
        .or_else(|| profile.get_non_empty("server"))
        .unwrap_or("localhost")
        .trim();

    if raw.is_empty() {
        return ("localhost".to_string(), fallback_port);
    }

    if let Some(rest) = raw.strip_prefix('[') {
        if let Some(close) = rest.find(']') {
            let host = &rest[..close];
            let tail = &rest[close + 1..];
            if let Some(port_str) = tail.strip_prefix(':') {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    return (host.to_string(), port);
                }
            }
            return (host.to_string(), fallback_port);
        }
    }

    if let Some((h, p)) = raw.rsplit_once(':') {
        if !h.contains(':') {
            if let Ok(port) = p.trim().parse::<u16>() {
                let host = if h.trim().is_empty() {
                    "localhost"
                } else {
                    h.trim()
                };
                return (host.to_string(), port);
            }
        }
    }

    (raw.to_string(), fallback_port)
}

#[derive(Debug, Default)]
pub struct PostgresProvider;

impl DatabaseProvider for PostgresProvider {
    fn id(&self) -> &'static str {
        "postgres"
    }

    fn display_name(&self) -> &'static str {
        "PostgreSQL"
    }

    fn dialect(&self) -> &'static str {
        "postgres"
    }

    fn settings(&self) -> &'static [ProviderSetting] {
        SETTINGS
    }

    fn editor_language(&self) -> Arc<dyn EditorLanguage> {
        Arc::new(SqlLanguage::postgres())
    }

    fn read_only_policy(&self) -> Option<Arc<dyn ReadOnlyStatementPolicy>> {
        Some(Arc::new(PostgresReadOnlyPolicy))
    }

    fn describe_endpoint(&self, profile: &ConnectionProfile) -> Option<String> {
        let (host, port) = parse_host_and_port(profile);
        Some(format!("PostgreSQL · {host}:{port}"))
    }

    fn describe_identity(&self, profile: &ConnectionProfile) -> Option<String> {
        profile.get_non_empty("user").map(str::to_string)
    }

    fn transport_warning(&self, profile: &ConnectionProfile) -> Option<&'static str> {
        let (host, _) = parse_host_and_port(profile);
        if is_local_host(Some(&host)) {
            return None;
        }
        let raw_mode = profile.get("sslMode");
        if raw_mode.map(str::trim).unwrap_or("").is_empty() {
            return None;
        }
        match PostgresSslMode::parse(raw_mode, false) {
            Ok(PostgresSslMode::Disable | PostgresSslMode::Allow | PostgresSslMode::Prefer) => {
                Some(UNVERIFIED_TLS_WARNING)
            }
            Ok(
                PostgresSslMode::Require
                | PostgresSslMode::VerifyCa
                | PostgresSslMode::VerifyFull,
            )
            | Err(_) => None,
        }
    }

    fn create_connection(&self, profile: ConnectionProfile) -> Box<dyn DatabaseConnection> {
        Box::new(PostgresConnection::new(profile))
    }
}

struct PhysicalSession {
    client: Client,
    notices: Arc<Mutex<Vec<String>>>,
    driver_task: JoinHandle<()>,
    use_tls: bool,
    tls_mode: PostgresSslMode,
}

impl PhysicalSession {
    fn drain_notices(&self) -> Vec<String> {
        let mut guard = self.notices.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    }

    fn close(self) {
        drop(self.client);
        self.driver_task.abort();
    }
}

pub struct PostgresConnection {
    profile: ConnectionProfile,
    state: ConnectionState,
    server_version: Option<String>,
    control_database: String,
    sessions: HashMap<String, PhysicalSession>,
}

impl PostgresConnection {
    pub fn new(profile: ConnectionProfile) -> Self {
        let control_database = profile
            .get_non_empty("database")
            .unwrap_or(DEFAULT_DATABASE)
            .to_string();
        Self {
            profile,
            state: ConnectionState::Disconnected,
            server_version: None,
            control_database,
            sessions: HashMap::new(),
        }
    }

    fn query_timeout(&self) -> Option<Duration> {
        let secs = self
            .profile
            .get_u64("queryTimeout", DEFAULT_QUERY_TIMEOUT_SECS);
        if secs == 0 {
            None
        } else {
            Some(Duration::from_secs(secs))
        }
    }

    fn connect_timeout(&self) -> Duration {
        let secs = self
            .profile
            .get_u64("connectTimeout", DEFAULT_CONNECT_TIMEOUT_SECS)
            .max(1);
        Duration::from_secs(secs)
    }

    fn build_config(&self, database: &str) -> Result<(Config, PostgresSslMode)> {
        let (host, port) = parse_host_and_port(&self.profile);
        let ssl_mode =
            PostgresSslMode::parse(self.profile.get("sslMode"), is_local_host(Some(&host)))?;

        let mut cfg = Config::new();
        cfg.host(&host);
        cfg.port(port);
        cfg.dbname(database);
        cfg.application_name("Larik");
        cfg.connect_timeout(self.connect_timeout());

        let user = self.profile.get_non_empty("user").unwrap_or(DEFAULT_USER);
        cfg.user(user);

        if let Some(password) = self.profile.get("password").filter(|p| !p.is_empty()) {
            cfg.password(password);
        }

        match ssl_mode {
            PostgresSslMode::Disable => {
                cfg.ssl_mode(tokio_postgres::config::SslMode::Disable);
            }
            PostgresSslMode::Allow | PostgresSslMode::Prefer => {
                cfg.ssl_mode(tokio_postgres::config::SslMode::Prefer);
            }
            PostgresSslMode::Require
            | PostgresSslMode::VerifyCa
            | PostgresSslMode::VerifyFull => {
                cfg.ssl_mode(tokio_postgres::config::SslMode::Require);
            }
        }

        Ok((cfg, ssl_mode))
    }

    async fn connect_physical(&self, database: &str) -> Result<PhysicalSession> {
        let (cfg, ssl_mode) = self.build_config(database)?;
        let timeout = self.connect_timeout();

        match ssl_mode {
            PostgresSslMode::Disable => {
                connect_no_tls(&cfg, ssl_mode, timeout).await
            }
            PostgresSslMode::Allow => match connect_no_tls(&cfg, ssl_mode, timeout).await {
                Ok(session) => Ok(session),
                Err(_) => connect_with_tls(&cfg, ssl_mode, timeout).await,
            },
            PostgresSslMode::Prefer => match connect_with_tls(&cfg, ssl_mode, timeout).await {
                Ok(session) => Ok(session),
                Err(_) => {
                    let mut plain = cfg.clone();
                    plain.ssl_mode(tokio_postgres::config::SslMode::Disable);
                    connect_no_tls(&plain, ssl_mode, timeout).await
                }
            },
            PostgresSslMode::Require
            | PostgresSslMode::VerifyCa
            | PostgresSslMode::VerifyFull => connect_with_tls(&cfg, ssl_mode, timeout).await,
        }
    }

    async fn ensure_session(&mut self, database: Option<&str>) -> Result<&mut PhysicalSession> {
        let target_db = database
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.control_database)
            .to_string();

        let needs_reconnect = self
            .sessions
            .get(&target_db)
            .is_none_or(|s| s.client.is_closed());

        if needs_reconnect {
            if let Some(old) = self.sessions.remove(&target_db) {
                old.close();
            }
            let session = self.connect_physical(&target_db).await?;
            self.sessions.insert(target_db.clone(), session);
        }

        self.sessions
            .get_mut(&target_db)
            .ok_or_else(|| anyhow!("PostgreSQL session for database '{target_db}' is unavailable."))
    }

    async fn cancel_query_best_effort(session: &PhysicalSession) {
        let cancel_token = session.client.cancel_token();
        if session.use_tls {
            if let Ok(tls) = build_tls_connector(session.tls_mode) {
                let _ = tokio::time::timeout(
                    Duration::from_secs(3),
                    cancel_token.cancel_query(tls),
                )
                .await;
                return;
            }
        }
        let _ =
            tokio::time::timeout(Duration::from_secs(3), cancel_token.cancel_query(NoTls)).await;
    }
}

fn build_tls_connector(mode: PostgresSslMode) -> Result<MakeTlsConnector> {
    let mut builder = TlsConnector::builder();
    match mode {
        PostgresSslMode::VerifyFull => {
        }
        PostgresSslMode::VerifyCa => {
            builder.danger_accept_invalid_hostnames(true);
        }
        PostgresSslMode::Require | PostgresSslMode::Prefer | PostgresSslMode::Allow => {
            builder.danger_accept_invalid_certs(true);
            builder.danger_accept_invalid_hostnames(true);
        }
        PostgresSslMode::Disable => {}
    }
    let connector = builder.build().context("Failed to initialize TLS connector")?;
    Ok(MakeTlsConnector::new(connector))
}

async fn connect_no_tls(
    cfg: &Config,
    ssl_mode: PostgresSslMode,
    timeout: Duration,
) -> Result<PhysicalSession> {
    let (client, mut connection) = tokio::time::timeout(timeout, cfg.connect(NoTls))
        .await
        .map_err(|_| anyhow!("Timed out connecting to PostgreSQL after {}s.", timeout.as_secs()))?
        .map_err(format_pg_error_anyhow)?;

    let notices = Arc::new(Mutex::new(Vec::new()));
    let notices_bg = Arc::clone(&notices);
    let driver_task = tokio::spawn(async move {
        while let Some(msg) = poll_fn(|cx| connection.poll_message(cx)).await {
            match msg {
                Ok(AsyncMessage::Notice(err)) => {
                    if let Ok(mut guard) = notices_bg.lock() {
                        guard.push(format_notice(&err));
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    Ok(PhysicalSession {
        client,
        notices,
        driver_task,
        use_tls: false,
        tls_mode: ssl_mode,
    })
}

async fn connect_with_tls(
    cfg: &Config,
    ssl_mode: PostgresSslMode,
    timeout: Duration,
) -> Result<PhysicalSession> {
    let tls = build_tls_connector(ssl_mode)?;
    let (client, mut connection) = tokio::time::timeout(timeout, cfg.connect(tls))
        .await
        .map_err(|_| anyhow!("Timed out connecting to PostgreSQL after {}s.", timeout.as_secs()))?
        .map_err(format_pg_error_anyhow)?;

    let notices = Arc::new(Mutex::new(Vec::new()));
    let notices_bg = Arc::clone(&notices);
    let driver_task = tokio::spawn(async move {
        while let Some(msg) = poll_fn(|cx| connection.poll_message(cx)).await {
            match msg {
                Ok(AsyncMessage::Notice(err)) => {
                    if let Ok(mut guard) = notices_bg.lock() {
                        guard.push(format_notice(&err));
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    Ok(PhysicalSession {
        client,
        notices,
        driver_task,
        use_tls: true,
        tls_mode: ssl_mode,
    })
}

fn format_notice(err: &tokio_postgres::error::DbError) -> String {
    let sev = err.severity();
    if sev.is_empty() || sev.eq_ignore_ascii_case("NOTICE") {
        err.message().to_string()
    } else {
        format!("{sev}: {}", err.message())
    }
}

fn format_pg_error(err: &PgError) -> String {
    if let Some(db) = err.as_db_error() {
        let mut msg = db.message().to_string();
        if let Some(detail) = db.detail().map(str::trim).filter(|s| !s.is_empty()) {
            msg.push('\n');
            msg.push_str(detail);
        }
        if let Some(hint) = db.hint().map(str::trim).filter(|s| !s.is_empty()) {
            msg.push_str("\nHint: ");
            msg.push_str(hint);
        }
        if let Some(where_) = db.where_().map(str::trim).filter(|s| !s.is_empty()) {
            msg.push_str("\nWhere: ");
            msg.push_str(where_);
        }
        return msg;
    }
    err.to_string()
}

fn format_pg_error_anyhow(err: PgError) -> anyhow::Error {
    anyhow!("{}", format_pg_error(&err))
}

fn is_permission_denied(err: &PgError) -> bool {
    err.code()
        .is_some_and(|code| code.code() == "42501")
}

fn simple_messages_to_results(
    messages: Vec<SimpleQueryMessage>,
    max_rows: usize,
    elapsed: Duration,
    database: Option<String>,
    statement: Option<String>,
    notices: Vec<String>,
) -> Vec<QueryResult> {
    let mut results = Vec::new();
    let mut cur_columns: Option<Vec<QueryColumn>> = None;
    let mut cur_rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut cur_truncated = false;
    let mut notices_opt = Some(notices);

    for msg in messages {
        match msg {
            SimpleQueryMessage::RowDescription(cols) => {
                if let Some(prev_cols) = cur_columns.take() {
                    let row_count = cur_rows.len();
                    results.push(QueryResult {
                        columns: prev_cols,
                        rows: std::mem::take(&mut cur_rows),
                        records_affected: None,
                        message: format_row_message(row_count, cur_truncated, max_rows),
                        is_error: false,
                        elapsed,
                        database: database.clone(),
                        is_truncated: cur_truncated,
                        row_limit: cur_truncated.then_some(max_rows),
                        messages: notices_opt.take().unwrap_or_default(),
                        statement: statement.clone(),
                    });
                    cur_truncated = false;
                }
                cur_columns = Some(
                    cols.iter()
                        .map(|c| QueryColumn {
                            name: c.name().to_string(),
                            data_type: None,
                        })
                        .collect(),
                );
            }
            SimpleQueryMessage::Row(row) => {
                if cur_columns.is_none() {
                    cur_columns = Some(
                        row.columns()
                            .iter()
                            .map(|c| QueryColumn {
                                name: c.name().to_string(),
                                data_type: None,
                            })
                            .collect(),
                    );
                }
                if cur_rows.len() < max_rows {
                    let col_len = row.len();
                    let mut values = Vec::with_capacity(col_len);
                    for i in 0..col_len {
                        values.push(row.get(i).map(str::to_string));
                    }
                    cur_rows.push(values);
                } else {
                    cur_truncated = true;
                }
            }
            SimpleQueryMessage::CommandComplete(affected) => {
                if let Some(cols) = cur_columns.take() {
                    let row_count = cur_rows.len();
                    results.push(QueryResult {
                        columns: cols,
                        rows: std::mem::take(&mut cur_rows),
                        records_affected: None,
                        message: format_row_message(row_count, cur_truncated, max_rows),
                        is_error: false,
                        elapsed,
                        database: database.clone(),
                        is_truncated: cur_truncated,
                        row_limit: cur_truncated.then_some(max_rows),
                        messages: notices_opt.take().unwrap_or_default(),
                        statement: statement.clone(),
                    });
                    cur_truncated = false;
                } else {
                    let msg_text = if affected > 0 {
                        format!("{affected} row(s) affected.")
                    } else {
                        "Command completed.".to_string()
                    };
                    results.push(QueryResult {
                        columns: Vec::new(),
                        rows: Vec::new(),
                        records_affected: Some(affected as i64),
                        message: msg_text,
                        is_error: false,
                        elapsed,
                        database: database.clone(),
                        is_truncated: false,
                        row_limit: None,
                        messages: notices_opt.take().unwrap_or_default(),
                        statement: statement.clone(),
                    });
                }
            }
            _ => {}
        }
    }

    if let Some(cols) = cur_columns.take() {
        let row_count = cur_rows.len();
        results.push(QueryResult {
            columns: cols,
            rows: cur_rows,
            records_affected: None,
            message: format_row_message(row_count, cur_truncated, max_rows),
            is_error: false,
            elapsed,
            database,
            is_truncated: cur_truncated,
            row_limit: cur_truncated.then_some(max_rows),
            messages: notices_opt.take().unwrap_or_default(),
            statement,
        });
    } else if results.is_empty() {
        results.push(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            records_affected: None,
            message: "Command completed.".to_string(),
            is_error: false,
            elapsed,
            database,
            is_truncated: false,
            row_limit: None,
            messages: notices_opt.take().unwrap_or_default(),
            statement,
        });
    }

    results
}

fn format_row_message(row_count: usize, is_truncated: bool, max_rows: usize) -> String {
    if is_truncated {
        format!("Showing first {max_rows} rows (truncated).")
    } else if row_count == 1 {
        "1 row returned.".to_string()
    } else {
        format!("{row_count} rows returned.")
    }
}

#[async_trait]
impl DatabaseConnection for PostgresConnection {
    fn state(&self) -> ConnectionState {
        self.state
    }

    fn server_version(&self) -> Option<String> {
        self.server_version.clone()
    }

    async fn open(&mut self) -> Result<()> {
        self.state = ConnectionState::Connecting;
        let control_db = self.control_database.clone();
        match self.connect_physical(&control_db).await {
            Ok(session) => {
                let ver = match session.client.simple_query("SHOW server_version").await {
                    Ok(msgs) => msgs.into_iter().find_map(|m| match m {
                        SimpleQueryMessage::Row(r) => r
                            .get(0)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(|v| format!("PostgreSQL {v}")),
                        _ => None,
                    }),
                    Err(_) => None,
                };
                self.server_version = ver;
                if let Some(old) = self.sessions.insert(control_db, session) {
                    old.close();
                }
                self.state = ConnectionState::Connected;
                Ok(())
            }
            Err(err) => {
                self.state = ConnectionState::Faulted;
                Err(err)
            }
        }
    }

    async fn close(&mut self) -> Result<()> {
        let sessions = std::mem::take(&mut self.sessions);
        for (_, session) in sessions {
            session.close();
        }
        self.state = ConnectionState::Disconnected;
        Ok(())
    }

    async fn list_databases(&mut self) -> Result<Vec<DatabaseInfo>> {
        let control_db = self.control_database.clone();
        let session = self.ensure_session(Some(&control_db)).await?;

        let sql = "SELECT datname, \
            datallowconn AND has_database_privilege(current_user, datname, 'CONNECT') AS can_connect \
            FROM pg_catalog.pg_database \
            WHERE NOT datistemplate \
            ORDER BY datname;";

        let rows = match session.client.query(sql, &[]).await {
            Ok(rows) => rows,
            Err(err) if is_permission_denied(&err) => {
                return Ok(vec![DatabaseInfo::accessible(control_db)]);
            }
            Err(err) => return Err(format_pg_error_anyhow(err)),
        };

        let mut dbs = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.get(0);
            let accessible: bool = row.get(1);
            dbs.push(DatabaseInfo {
                name,
                is_accessible: accessible,
            });
        }
        DatabaseInfo::sort_display_order(&mut dbs);
        Ok(dbs)
    }

    async fn get_schema(&mut self, database: Option<&str>) -> Result<Vec<TableSchema>> {
        let session = self.ensure_session(database).await?;

        let columns_sql = r#"
SELECT
    n.nspname AS table_schema,
    c.relname AS table_name,
    c.relkind::text AS relkind,
    a.attname AS column_name,
    pg_catalog.format_type(a.atttypid, a.atttypmod) AS data_type,
    NOT a.attnotnull AS is_nullable,
    (a.attidentity <> '') AS is_identity,
    (a.attgenerated <> '') AS is_computed
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid
WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
  AND a.attnum > 0
  AND NOT a.attisdropped
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND n.nspname NOT LIKE 'pg_toast%'
  AND n.nspname NOT LIKE 'pg_temp_%'
ORDER BY n.nspname, c.relname, a.attnum;
"#;

        let col_rows = match session.client.query(columns_sql, &[]).await {
            Ok(rows) => rows,
            Err(err) if is_permission_denied(&err) => return Ok(Vec::new()),
            Err(err) => return Err(format_pg_error_anyhow(err)),
        };

        let mut tables_map: BTreeMap<(String, String), TableSchema> = BTreeMap::new();
        for row in col_rows {
            let schema: String = row.get(0);
            let name: String = row.get(1);
            let relkind: String = row.get(2);
            let col_name: String = row.get(3);
            let data_type: String = row.get(4);
            let nullable: bool = row.get(5);
            let is_identity: bool = row.get(6);
            let is_computed: bool = row.get(7);

            let kind = match relkind.as_str() {
                "v" | "m" => TableKind::View,
                _ => TableKind::Table,
            };

            let entry = tables_map
                .entry((schema.clone(), name.clone()))
                .or_insert_with(|| TableSchema {
                    schema,
                    name,
                    kind,
                    columns: Vec::new(),
                    primary_key: Vec::new(),
                    foreign_keys: Vec::new(),
                });

            entry.columns.push(ColumnSchema {
                name: col_name,
                data_type: Some(data_type),
                nullable,
                is_identity,
                is_computed,
            });
        }

        if tables_map.is_empty() {
            return Ok(Vec::new());
        }

        let pk_sql = r#"
SELECT
    n.nspname AS table_schema,
    c.relname AS table_name,
    a.attname AS column_name
FROM pg_catalog.pg_constraint con
JOIN pg_catalog.pg_class c ON c.oid = con.conrelid
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) ON TRUE
JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum
WHERE con.contype = 'p'
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
ORDER BY n.nspname, c.relname, k.ord;
"#;

        if let Ok(pk_rows) = session.client.query(pk_sql, &[]).await {
            for row in pk_rows {
                let schema: String = row.get(0);
                let name: String = row.get(1);
                let col: String = row.get(2);
                if let Some(tbl) = tables_map.get_mut(&(schema, name)) {
                    tbl.primary_key.push(col);
                }
            }
        }

        let fk_sql = r#"
SELECT
    con.conname AS constraint_name,
    n.nspname AS table_schema,
    c.relname AS table_name,
    a.attname AS column_name,
    rn.nspname AS ref_schema,
    rc.relname AS ref_table,
    ra.attname AS ref_column
FROM pg_catalog.pg_constraint con
JOIN pg_catalog.pg_class c ON c.oid = con.conrelid
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
JOIN pg_catalog.pg_class rc ON rc.oid = con.confrelid
JOIN pg_catalog.pg_namespace rn ON rn.oid = rc.relnamespace
JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS fk(attnum, ord) ON TRUE
JOIN LATERAL unnest(con.confkey) WITH ORDINALITY AS rk(attnum, ord) ON rk.ord = fk.ord
JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid AND a.attnum = fk.attnum
JOIN pg_catalog.pg_attribute ra ON ra.attrelid = rc.oid AND ra.attnum = rk.attnum
WHERE con.contype = 'f'
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
ORDER BY n.nspname, c.relname, con.conname, fk.ord;
"#;

        if let Ok(fk_rows) = session.client.query(fk_sql, &[]).await {
            for row in fk_rows {
                let constraint: String = row.get(0);
                let schema: String = row.get(1);
                let name: String = row.get(2);
                let column: String = row.get(3);
                let ref_schema: String = row.get(4);
                let ref_table: String = row.get(5);
                let ref_column: String = row.get(6);
                if let Some(tbl) = tables_map.get_mut(&(schema, name)) {
                    tbl.foreign_keys.push(ForeignKeySchema {
                        constraint,
                        column,
                        referenced_schema: Some(ref_schema),
                        referenced_table: ref_table,
                        referenced_column: ref_column,
                    });
                }
            }
        }

        Ok(tables_map.into_values().collect())
    }

    async fn get_routines(&mut self, database: Option<&str>) -> Result<Vec<RoutineSchema>> {
        let session = self.ensure_session(database).await?;

        let sql = r#"
SELECT
    r.routine_schema,
    r.routine_name,
    r.specific_name,
    r.routine_type,
    p.parameter_name,
    p.data_type,
    p.udt_name,
    p.ordinal_position
FROM information_schema.routines r
LEFT JOIN information_schema.parameters p
    ON p.specific_schema = r.specific_schema
   AND p.specific_name = r.specific_name
   AND p.parameter_mode IN ('IN', 'INOUT')
WHERE r.routine_schema NOT IN ('pg_catalog', 'information_schema')
ORDER BY r.routine_schema, r.routine_name, r.specific_name, p.ordinal_position;
"#;

        let rows = match session.client.query(sql, &[]).await {
            Ok(rows) => rows,
            Err(err) if is_permission_denied(&err) => return Ok(Vec::new()),
            Err(err) => return Err(format_pg_error_anyhow(err)),
        };

        let mut by_name: BTreeMap<(String, String), (String, RoutineSchema)> = BTreeMap::new();
        for row in rows {
            let schema: String = row.get(0);
            let name: String = row.get(1);
            let specific: String = row.get(2);
            let routine_type: String = row.get(3);
            let param_name: Option<String> = row.get(4);
            let param_data_type: Option<String> = row.get(5);
            let param_udt_name: Option<String> = row.get(6);
            let param_ord: Option<i32> = row.get(7);

            let key = (schema.clone(), name.clone());
            let entry = by_name.entry(key).or_insert_with(|| {
                let kind = if routine_type.eq_ignore_ascii_case("PROCEDURE") {
                    RoutineKind::Procedure
                } else {
                    RoutineKind::Function
                };
                (
                    specific.clone(),
                    RoutineSchema {
                        schema,
                        name,
                        kind,
                        parameters: Vec::new(),
                    },
                )
            });

            if entry.0 != specific {
                continue;
            }

            if param_ord.is_some() {
                let display_name = param_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("${}", param_ord.unwrap_or(1)));
                let data_type = normalize_pg_routine_param_type(
                    param_data_type.as_deref(),
                    param_udt_name.as_deref(),
                );
                entry.1.parameters.push(RoutineParameter {
                    name: display_name,
                    data_type,
                });
            }
        }

        Ok(by_name.into_values().map(|(_, r)| r).collect())
    }

    async fn estimate_row_count(
        &mut self,
        schema: Option<&str>,
        table: &str,
        database: Option<&str>,
    ) -> Option<i64> {
        let session = self.ensure_session(database).await.ok()?;
        let sql = "SELECT c.reltuples::bigint \
            FROM pg_catalog.pg_class c \
            JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
            WHERE n.nspname = COALESCE(NULLIF($1, ''), current_schema()) \
              AND c.relname = $2 \
              AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
            ORDER BY c.oid \
            LIMIT 1;";
        let schema_param = schema.unwrap_or("");
        let row = session
            .client
            .query_opt(sql, &[&schema_param, &table])
            .await
            .ok()??;
        let est: i64 = row.try_get(0).ok()?;
        (est >= 0).then_some(est)
    }

    async fn execute_batch(
        &mut self,
        sql: &str,
        database: Option<&str>,
        options: &QueryExecutionOptions,
    ) -> Result<Vec<QueryResult>> {
        let target_db = database
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.control_database)
            .to_string();
        let max_rows = options.effective_max_rows();
        let read_only = options.read_only == Some(true);
        let timeout = self.query_timeout();

        let lang = SqlLanguage::postgres();
        let statements = lang.split_statements(sql);
        if statements.is_empty() {
            return Ok(vec![QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: None,
                message: "Nothing to run.".to_string(),
                is_error: false,
                elapsed: Duration::ZERO,
                database: Some(target_db),
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: None,
            }]);
        }

        self.ensure_session(Some(&target_db)).await?;

        let mut all_results = Vec::new();
        let mut drop_session = false;

        if read_only {
            let session = self
                .sessions
                .get(&target_db)
                .ok_or_else(|| anyhow!("PostgreSQL session unavailable."))?;
            let start = Instant::now();
            if let Err(err) = session.client.batch_execute("BEGIN TRANSACTION READ ONLY").await {
                return Ok(vec![QueryResult::error(
                    format_pg_error(&err),
                    start.elapsed(),
                    Some(target_db),
                )]);
            }
        }

        for stmt in statements {
            let start = Instant::now();
            let Some(session) = self.sessions.get(&target_db) else {
                all_results.push(QueryResult::error(
                    "PostgreSQL connection closed unexpectedly.",
                    start.elapsed(),
                    Some(target_db.clone()),
                ));
                break;
            };
            let _ = session.drain_notices();

            let exec_outcome = match timeout {
                Some(limit) => {
                    match tokio::time::timeout(limit, session.client.simple_query(&stmt.text)).await
                    {
                        Ok(res) => Ok(res),
                        Err(_) => {
                            Self::cancel_query_best_effort(session).await;
                            drop_session = true;
                            Err(format!(
                                "Query timed out after {} second(s).",
                                limit.as_secs()
                            ))
                        }
                    }
                }
                None => Ok(session.client.simple_query(&stmt.text).await),
            };

            let notices = session.drain_notices();

            match exec_outcome {
                Ok(Ok(messages)) => {
                    let mut stmt_results = simple_messages_to_results(
                        messages,
                        max_rows,
                        start.elapsed(),
                        Some(target_db.clone()),
                        Some(stmt.text),
                        notices,
                    );
                    all_results.append(&mut stmt_results);
                }
                Ok(Err(pg_err)) => {
                    let mut res = QueryResult::error(
                        format_pg_error(&pg_err),
                        start.elapsed(),
                        Some(target_db.clone()),
                    );
                    res.messages = notices;
                    res.statement = Some(stmt.text);
                    all_results.push(res);
                    break;
                }
                Err(timeout_msg) => {
                    let mut res = QueryResult::error(
                        timeout_msg,
                        start.elapsed(),
                        Some(target_db.clone()),
                    );
                    res.messages = notices;
                    res.statement = Some(stmt.text);
                    all_results.push(res);
                    break;
                }
            }
        }

        if read_only && !drop_session {
            if let Some(session) = self.sessions.get(&target_db) {
                if session.client.batch_execute("ROLLBACK").await.is_err() {
                    drop_session = true;
                }
            }
        }

        if drop_session {
            if let Some(old) = self.sessions.remove(&target_db) {
                old.close();
            }
        }

        Ok(all_results)
    }
}

fn normalize_pg_routine_param_type(
    data_type: Option<&str>,
    udt_name: Option<&str>,
) -> Option<String> {
    let dt = data_type.map(str::trim).filter(|s| !s.is_empty());
    let udt = udt_name.map(str::trim).filter(|s| !s.is_empty());
    match (dt, udt) {
        (Some(d), Some(u)) if d.eq_ignore_ascii_case("ARRAY") => {
            Some(format!("{}[]", u.trim_start_matches('_')))
        }
        (Some(d), Some(u)) if d.eq_ignore_ascii_case("USER-DEFINED") => Some(u.to_string()),
        (Some(d), _) => Some(d.to_string()),
        (None, Some(u)) => Some(u.to_string()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_metadata_and_settings_match_reference() {
        let provider = PostgresProvider;
        assert_eq!(provider.id(), "postgres");
        assert_eq!(provider.display_name(), "PostgreSQL");
        assert_eq!(provider.dialect(), "postgres");
        assert!(provider.read_only_policy().is_some());
        assert_eq!(provider.password_keys(), vec!["password"]);

        let keys: Vec<&str> = provider.settings().iter().map(|s| s.key).collect();
        assert_eq!(
            keys,
            vec![
                "host",
                "port",
                "database",
                "user",
                "password",
                "sslMode",
                "connectTimeout",
                "queryTimeout"
            ]
        );

        let profile = provider.default_profile();
        assert_eq!(
            provider.describe_endpoint(&profile).as_deref(),
            Some("PostgreSQL · localhost:5432")
        );
        assert_eq!(
            provider.describe_identity(&profile).as_deref(),
            Some("postgres")
        );
        assert_eq!(provider.transport_warning(&profile), None);
    }

    #[test]
    fn localhost_detection_covers_loopback_and_sockets() {
        assert!(is_local_host(Some("localhost")));
        assert!(is_local_host(Some("LOCALHOST")));
        assert!(is_local_host(Some("127.0.0.1")));
        assert!(is_local_host(Some("::1")));
        assert!(is_local_host(Some("[::1]")));
        assert!(is_local_host(Some("/var/run/postgresql")));
        assert!(is_local_host(Some(r"\\.\pipe\pg")));
        assert!(!is_local_host(Some("db.example.com")));
        assert!(!is_local_host(Some("10.0.0.5")));
        assert!(!is_local_host(None));
    }

    #[test]
    fn ssl_mode_defaults_and_rejects_typos() {
        assert_eq!(
            PostgresSslMode::parse(None, true).unwrap(),
            PostgresSslMode::Prefer
        );
        assert_eq!(
            PostgresSslMode::parse(Some(""), false).unwrap(),
            PostgresSslMode::VerifyFull
        );
        assert_eq!(
            PostgresSslMode::parse(Some("verify-ca"), false).unwrap(),
            PostgresSslMode::VerifyCa
        );
        assert_eq!(
            PostgresSslMode::parse(Some("VerifyFull"), false).unwrap(),
            PostgresSslMode::VerifyFull
        );
        assert!(PostgresSslMode::parse(Some("VerifyFul"), false).is_err());
    }

    #[test]
    fn transport_warning_matches_ssl_policy() {
        let provider = PostgresProvider;
        let mut remote = provider.default_profile();
        remote.set("host", "db.prod.example.com");

        assert_eq!(provider.transport_warning(&remote), None);

        for weak in ["Disable", "Allow", "Prefer"] {
            remote.set("sslMode", weak);
            assert!(
                provider.transport_warning(&remote).is_some(),
                "expected warning for {weak}"
            );
        }

        for strong in ["Require", "VerifyCA", "VerifyFull"] {
            remote.set("sslMode", strong);
            assert_eq!(
                provider.transport_warning(&remote),
                None,
                "unexpected warning for {strong}"
            );
        }

        remote.set("sslMode", "Bogus");
        assert_eq!(provider.transport_warning(&remote), None);

        let mut local = provider.default_profile();
        local.set("host", "127.0.0.1");
        local.set("sslMode", "Disable");
        assert_eq!(provider.transport_warning(&local), None);
    }

    #[test]
    fn parse_host_and_port_handles_ipv6_embedded_port_and_legacy_server_key() {
        let mut p = ConnectionProfile::new("postgres");
        p.set("server", "pg.internal:5433");
        assert_eq!(parse_host_and_port(&p), ("pg.internal".to_string(), 5433));

        let mut ipv6 = ConnectionProfile::new("postgres");
        ipv6.set("host", "[::1]:5439");
        assert_eq!(parse_host_and_port(&ipv6), ("::1".to_string(), 5439));

        let mut bare_ipv6 = ConnectionProfile::new("postgres");
        bare_ipv6.set("host", "::1");
        bare_ipv6.set("port", "5434");
        assert_eq!(parse_host_and_port(&bare_ipv6), ("::1".to_string(), 5434));
    }

    #[test]
    fn editor_language_uses_double_quotes_and_limit_preview() {
        let provider = PostgresProvider;
        let lang = provider.editor_language();
        assert_eq!(lang.quote_identifier("users"), "\"users\"");
        assert_eq!(
            lang.preview_statement(Some("public"), "orders", 100),
            "SELECT * FROM \"public\".\"orders\" LIMIT 100;"
        );
    }

    #[test]
    fn routine_param_types_normalize_arrays_and_user_defined() {
        assert_eq!(
            normalize_pg_routine_param_type(Some("ARRAY"), Some("_int4")).as_deref(),
            Some("int4[]")
        );
        assert_eq!(
            normalize_pg_routine_param_type(Some("USER-DEFINED"), Some("order_status")).as_deref(),
            Some("order_status")
        );
        assert_eq!(
            normalize_pg_routine_param_type(Some("integer"), Some("int4")).as_deref(),
            Some("integer")
        );
    }
}
