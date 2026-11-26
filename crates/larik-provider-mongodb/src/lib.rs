pub mod commands;
pub mod compiler;
pub mod dialect;
pub mod formatter;
pub mod read_only;
pub mod shell;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use larik_data::read_only_sql::ReadOnlyStatementPolicy;
use larik_data::{
    ColumnSchema, ConnectionProfile, ConnectionState, DatabaseConnection, DatabaseInfo,
    DatabaseProvider, EditorLanguage, ProviderSetting, ProviderSettingKind, QueryExecutionOptions,
    QueryResult, TableKind, TableSchema,
};
use mongodb::bson::{Bson, Document, doc};
use mongodb::options::{ClientOptions, Credential, ServerAddress};
use mongodb::Client;

use crate::commands::MongoCommand;
use crate::compiler::{TSqlMongoCompiler, pipeline_has_write_stage};
use crate::dialect::{MongoTSqlDialect, StatementKind, classify_statement};
use crate::formatter::bson_type_name;
use crate::read_only::MongoReadOnlyPolicy;
use crate::shell::{documents_to_query_result, execute_shell_statement};

const DEFAULT_PORT: u16 = 27017;
const DEFAULT_AUTH_SOURCE: &str = "admin";
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_QUERY_TIMEOUT_SECS: u64 = 60;
const SCHEMA_SAMPLE_LIMIT: i64 = 200;
const MAX_NESTED_FIELD_DEPTH: usize = 4;

const SETTINGS: &[ProviderSetting] = &[
    ProviderSetting {
        key: "connectionString",
        label: "Connection string",
        kind: ProviderSettingKind::Password,
        required: false,
        default_value: None,
        placeholder: Some("mongodb+srv://user:password@cluster/database"),
        hint: Some(
            "Optional. Takes precedence over Host/Port when set (recommended for Atlas SRV, TLS, or replica sets). Stored in Windows Credential Manager.",
        ),
        is_advanced: false,
    },
    ProviderSetting {
        key: "host",
        label: "Host",
        kind: ProviderSettingKind::Text,
        required: false,
        default_value: Some("localhost"),
        placeholder: Some("db.example.com, or db.example.com:27018"),
        hint: Some("Ignored when a connection string is set. host:port overrides Port."),
        is_advanced: false,
    },
    ProviderSetting {
        key: "port",
        label: "Port",
        kind: ProviderSettingKind::Number,
        required: false,
        default_value: Some("27017"),
        placeholder: None,
        hint: None,
        is_advanced: true,
    },
    ProviderSetting {
        key: "database",
        label: "Database",
        kind: ProviderSettingKind::Text,
        required: false,
        default_value: None,
        placeholder: Some("the default database to start in"),
        hint: None,
        is_advanced: false,
    },
    ProviderSetting {
        key: "authSource",
        label: "Auth source",
        kind: ProviderSettingKind::Text,
        required: false,
        default_value: Some("admin"),
        placeholder: Some("admin"),
        hint: None,
        is_advanced: true,
    },
    ProviderSetting {
        key: "user",
        label: "Username",
        kind: ProviderSettingKind::Text,
        required: false,
        default_value: None,
        placeholder: None,
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
        key: "directConnection",
        label: "Direct connection",
        kind: ProviderSettingKind::Boolean,
        required: false,
        default_value: Some("true"),
        placeholder: None,
        hint: Some("Connect directly to a single host without replica-set discovery."),
        is_advanced: true,
    },
    ProviderSetting {
        key: "connectTimeout",
        label: "Connect timeout (seconds)",
        kind: ProviderSettingKind::Number,
        required: false,
        default_value: Some("10"),
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
        hint: Some("How long a command may run before timing out. 0 waits indefinitely."),
        is_advanced: true,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MongoUriSummary {
    pub is_srv: bool,
    pub host_display: String,
    pub username: Option<String>,
    pub database: Option<String>,
}

pub fn parse_mongo_uri_summary(uri: &str) -> Option<MongoUriSummary> {
    let trimmed = uri.trim();
    let (is_srv, rest) = if let Some(r) = trimmed.strip_prefix("mongodb+srv://") {
        (true, r)
    } else if let Some(r) = trimmed.strip_prefix("mongodb://") {
        (false, r)
    } else {
        return None;
    };

    let (authority, path_and_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx + 1..]),
        None => match rest.find('?') {
            Some(idx) => (&rest[..idx], ""),
            None => (rest, ""),
        },
    };

    let (username, hosts_part) = match authority.rsplit_once('@') {
        Some((user_info, hosts)) => {
            let raw_user = user_info.split(':').next().unwrap_or("").trim();
            let user = (!raw_user.is_empty()).then(|| percent_decode_simple(raw_user));
            (user, hosts)
        }
        None => (None, authority),
    };

    let first_host = hosts_part.split(',').next().unwrap_or("").trim();
    if first_host.is_empty() {
        return None;
    }

    let host_display = if is_srv {
        first_host.to_string()
    } else if first_host.starts_with('[') {
        if first_host.contains("]:") {
            first_host.to_string()
        } else {
            format!("{first_host}:{DEFAULT_PORT}")
        }
    } else if first_host.contains(':') {
        first_host.to_string()
    } else {
        format!("{first_host}:{DEFAULT_PORT}")
    };

    let db_segment = path_and_query.split('?').next().unwrap_or("").trim();
    let database = (!db_segment.is_empty()).then(|| percent_decode_simple(db_segment));

    Some(MongoUriSummary {
        is_srv,
        host_display,
        username,
        database,
    })
}

fn percent_decode_simple(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn parse_host_and_port(profile: &ConnectionProfile) -> Result<(String, u16)> {
    let fallback_port = profile.get_u16("port").unwrap_or(DEFAULT_PORT);
    let raw = profile
        .get_non_empty("host")
        .or_else(|| profile.get_non_empty("server"))
        .unwrap_or("localhost")
        .trim();

    if raw.to_ascii_lowercase().starts_with("mongodb://")
        || raw.to_ascii_lowercase().starts_with("mongodb+srv://")
    {
        bail!(
            "A MongoDB URI was entered in the Host field. Move it to the Connection string field instead."
        );
    }

    if raw.is_empty() {
        return Ok(("localhost".to_string(), fallback_port));
    }

    if let Some(rest) = raw.strip_prefix('[') {
        if let Some(close) = rest.find(']') {
            let host = &rest[..close];
            let tail = &rest[close + 1..];
            if let Some(port_str) = tail.strip_prefix(':') {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    return Ok((host.to_string(), port));
                }
            }
            return Ok((host.to_string(), fallback_port));
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
                return Ok((host.to_string(), port));
            }
        }
    }

    Ok((raw.to_string(), fallback_port))
}

#[derive(Debug, Default)]
pub struct MongoProvider;

impl DatabaseProvider for MongoProvider {
    fn id(&self) -> &'static str {
        "mongodb"
    }

    fn display_name(&self) -> &'static str {
        "MongoDB"
    }

    fn dialect(&self) -> &'static str {
        "mongodb"
    }

    fn settings(&self) -> &'static [ProviderSetting] {
        SETTINGS
    }

    fn editor_language(&self) -> Arc<dyn EditorLanguage> {
        Arc::new(MongoTSqlDialect::new())
    }

    fn read_only_policy(&self) -> Option<Arc<dyn ReadOnlyStatementPolicy>> {
        Some(Arc::new(MongoReadOnlyPolicy))
    }

    fn describe_endpoint(&self, profile: &ConnectionProfile) -> Option<String> {
        if let Some(uri) = profile.get_non_empty("connectionString") {
            if let Some(summary) = parse_mongo_uri_summary(uri) {
                return Some(format!("MongoDB · {}", summary.host_display));
            }
            return Some("MongoDB · connection string".to_string());
        }
        match parse_host_and_port(profile) {
            Ok((host, port)) => Some(format!("MongoDB · {host}:{port}")),
            Err(_) => Some("MongoDB".to_string()),
        }
    }

    fn describe_identity(&self, profile: &ConnectionProfile) -> Option<String> {
        if let Some(uri) = profile.get_non_empty("connectionString") {
            if let Some(user) = parse_mongo_uri_summary(uri).and_then(|s| s.username) {
                return Some(user);
            }
        }
        profile.get_non_empty("user").map(str::to_string)
    }

    fn create_connection(&self, profile: ConnectionProfile) -> Box<dyn DatabaseConnection> {
        Box::new(MongoConnection::new(profile))
    }
}

pub struct MongoConnection {
    profile: ConnectionProfile,
    state: ConnectionState,
    server_version: Option<String>,
    default_database: String,
    client: Option<Client>,
    field_types_cache: HashMap<(String, String), HashMap<String, String>>,
}

impl MongoConnection {
    pub fn new(profile: ConnectionProfile) -> Self {
        let uri_db = profile
            .get_non_empty("connectionString")
            .and_then(parse_mongo_uri_summary)
            .and_then(|s| s.database);
        let default_database = profile
            .get_non_empty("database")
            .map(str::to_string)
            .or(uri_db)
            .unwrap_or_else(|| DEFAULT_AUTH_SOURCE.to_string());

        Self {
            profile,
            state: ConnectionState::Disconnected,
            server_version: None,
            default_database,
            client: None,
            field_types_cache: HashMap::new(),
        }
    }

    fn connect_timeout(&self) -> Duration {
        let secs = self
            .profile
            .get_u64("connectTimeout", DEFAULT_CONNECT_TIMEOUT_SECS)
            .max(1);
        Duration::from_secs(secs)
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

    async fn build_client_options(&self) -> Result<ClientOptions> {
        let timeout = self.connect_timeout();
        let mut options = if let Some(uri) = self.profile.get_non_empty("connectionString") {
            if !uri.starts_with("mongodb://") && !uri.starts_with("mongodb+srv://") {
                bail!("MongoDB connection string must start with 'mongodb://' or 'mongodb+srv://'.");
            }
            let mut opts = ClientOptions::parse(uri)
                .await
                .map_err(|e| anyhow!("Failed to parse MongoDB connection string: {e}"))?;

            let profile_user = self.profile.get_non_empty("user").map(str::to_string);
            let profile_pass = self
                .profile
                .get("password")
                .filter(|p| !p.is_empty())
                .map(str::to_string);
            let profile_auth_source = self
                .profile
                .get_non_empty("authSource")
                .map(str::to_string);

            match opts.credential.as_mut() {
                Some(cred) => {
                    if cred.password.is_none() {
                        cred.password = profile_pass;
                    }
                    if cred.source.is_none() {
                        cred.source = profile_auth_source;
                    }
                }
                None => {
                    if let Some(user) = profile_user {
                        opts.credential = Some(
                            Credential::builder()
                                .username(user)
                                .password(profile_pass)
                                .source(
                                    profile_auth_source
                                        .or_else(|| Some(DEFAULT_AUTH_SOURCE.to_string())),
                                )
                                .build(),
                        );
                    }
                }
            }
            opts
        } else {
            let (host, port) = parse_host_and_port(&self.profile)?;
            let direct = self.profile.get_bool("directConnection", true);
            let mut opts = ClientOptions::builder()
                .hosts(vec![ServerAddress::Tcp {
                    host,
                    port: Some(port),
                }])
                .direct_connection(direct)
                .build();

            if let Some(user) = self.profile.get_non_empty("user") {
                let password = self
                    .profile
                    .get("password")
                    .filter(|p| !p.is_empty())
                    .map(str::to_string);
                let auth_source = self
                    .profile
                    .get_non_empty("authSource")
                    .unwrap_or(DEFAULT_AUTH_SOURCE)
                    .to_string();
                opts.credential = Some(
                    Credential::builder()
                        .username(user.to_string())
                        .password(password)
                        .source(Some(auth_source))
                        .build(),
                );
            }
            opts
        };

        options.app_name = Some("Larik".to_string());
        if options.connect_timeout.is_none() {
            options.connect_timeout = Some(timeout);
        }
        if options.server_selection_timeout.is_none() {
            options.server_selection_timeout = Some(timeout);
        }
        Ok(options)
    }

    fn require_client(&self) -> Result<&Client> {
        self.client
            .as_ref()
            .ok_or_else(|| anyhow!("MongoDB connection is not open."))
    }

    fn resolve_database(&self, database: Option<&str>) -> String {
        database
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.default_database.clone())
    }

    fn cached_field_types(&self, database: &str, collection: &str) -> HashMap<String, String> {
        self.field_types_cache
            .get(&(database.to_ascii_lowercase(), collection.to_ascii_lowercase()))
            .cloned()
            .unwrap_or_default()
    }
}

fn collect_doc_fields(
    doc: &Document,
    prefix: &str,
    depth: usize,
    fields: &mut BTreeMap<String, String>,
) {
    if depth > MAX_NESTED_FIELD_DEPTH {
        return;
    }
    for (k, v) in doc {
        let path = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        let ty = bson_type_name(v);
        fields
            .entry(path.clone())
            .and_modify(|existing| {
                if existing == "null" && ty != "null" {
                    *existing = ty.to_string();
                }
            })
            .or_insert_with(|| ty.to_string());

        if let Bson::Document(sub) = v {
            collect_doc_fields(sub, &path, depth + 1, fields);
        }
    }
}

#[async_trait]
impl DatabaseConnection for MongoConnection {
    fn state(&self) -> ConnectionState {
        self.state
    }

    fn server_version(&self) -> Option<String> {
        self.server_version.clone()
    }

    async fn open(&mut self) -> Result<()> {
        self.state = ConnectionState::Connecting;
        let options = match self.build_client_options().await {
            Ok(opts) => opts,
            Err(err) => {
                self.state = ConnectionState::Faulted;
                return Err(err);
            }
        };

        let client = match Client::with_options(options) {
            Ok(c) => c,
            Err(err) => {
                self.state = ConnectionState::Faulted;
                return Err(anyhow!("Failed to initialize MongoDB client: {err}"));
            }
        };

        let ping_db = client.database(&self.default_database);
        let ping_res = tokio::time::timeout(
            self.connect_timeout(),
            ping_db.run_command(doc! { "ping": 1 }),
        )
        .await;

        let ping_ok = match ping_res {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e1)) => {
                if self.default_database != DEFAULT_AUTH_SOURCE {
                    client
                        .database(DEFAULT_AUTH_SOURCE)
                        .run_command(doc! { "ping": 1 })
                        .await
                        .map(|_| ())
                        .map_err(|_| anyhow!("MongoDB ping failed: {e1}"))
                } else {
                    Err(anyhow!("MongoDB ping failed: {e1}"))
                }
            }
            Err(_) => Err(anyhow!(
                "Timed out connecting to MongoDB after {}s.",
                self.connect_timeout().as_secs()
            )),
        };

        if let Err(err) = ping_ok {
            self.state = ConnectionState::Faulted;
            return Err(err);
        }

        let version = match ping_db.run_command(doc! { "buildInfo": 1 }).await {
            Ok(info) => info
                .get_str("version")
                .ok()
                .map(|v| format!("MongoDB {v}")),
            Err(_) => None,
        };
        self.server_version = version;
        self.client = Some(client);
        self.state = ConnectionState::Connected;
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.client = None;
        self.field_types_cache.clear();
        self.state = ConnectionState::Disconnected;
        Ok(())
    }

    async fn list_databases(&mut self) -> Result<Vec<DatabaseInfo>> {
        let client = self.require_client()?;
        match client.list_database_names().await {
            Ok(names) if !names.is_empty() => {
                let mut dbs: Vec<DatabaseInfo> = names
                    .into_iter()
                    .map(DatabaseInfo::accessible)
                    .collect();
                DatabaseInfo::sort_display_order(&mut dbs);
                Ok(dbs)
            }
            Ok(_) | Err(_) => {
                Ok(vec![DatabaseInfo::accessible(self.default_database.clone())])
            }
        }
    }

    async fn get_schema(&mut self, database: Option<&str>) -> Result<Vec<TableSchema>> {
        let db_name = self.resolve_database(database);
        let client = self.require_client()?.clone();
        let db = client.database(&db_name);

        let mut coll_names = db
            .list_collection_names()
            .await
            .with_context(|| format!("Failed to list collections in database '{db_name}'"))?;
        coll_names.retain(|n| !n.starts_with("system."));
        coll_names.sort_by_key(|a| a.to_ascii_lowercase());

        let mut tables = Vec::with_capacity(coll_names.len());

        for coll_name in coll_names {
            let coll = db.collection::<Document>(&coll_name);
            let mut fields: BTreeMap<String, String> = BTreeMap::new();
            let mut seen_docs = 0usize;

            if let Ok(mut cursor) = coll.find(doc! {}).limit(SCHEMA_SAMPLE_LIMIT).await {
                while let Ok(Some(doc)) = cursor.try_next().await {
                    collect_doc_fields(&doc, "", 0, &mut fields);
                    seen_docs += 1;
                }
            }

            if seen_docs == 0 && !fields.contains_key("_id") {
                fields.insert("_id".to_string(), "objectId".to_string());
            }

            let cache_map: HashMap<String, String> = fields
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            self.field_types_cache.insert(
                (db_name.to_ascii_lowercase(), coll_name.to_ascii_lowercase()),
                cache_map,
            );

            let mut columns = Vec::with_capacity(fields.len());
            if let Some(id_ty) = fields.remove("_id") {
                columns.push(ColumnSchema {
                    name: "_id".to_string(),
                    data_type: Some(id_ty),
                    nullable: false,
                    is_identity: false,
                    is_computed: false,
                });
            }
            for (name, ty) in fields {
                columns.push(ColumnSchema::simple(name, Some(ty), true));
            }

            tables.push(TableSchema {
                schema: String::new(),
                name: coll_name,
                kind: TableKind::Table,
                columns,
                primary_key: vec!["_id".to_string()],
                foreign_keys: Vec::new(),
            });
        }

        Ok(tables)
    }

    async fn estimate_row_count(
        &mut self,
        _schema: Option<&str>,
        table: &str,
        database: Option<&str>,
    ) -> Option<i64> {
        let client = self.client.as_ref()?;
        let db_name = self.resolve_database(database);
        let coll = client.database(&db_name).collection::<Document>(table);
        let count = coll.estimated_document_count().await.ok()?;
        i64::try_from(count).ok()
    }

    async fn execute_batch(
        &mut self,
        sql: &str,
        database: Option<&str>,
        options: &QueryExecutionOptions,
    ) -> Result<Vec<QueryResult>> {
        let client = self.require_client()?.clone();
        let mut active_db = self.resolve_database(database);
        let max_rows = options.effective_max_rows();
        let read_only = options.read_only == Some(true);
        let timeout = self.query_timeout();

        let dialect = MongoTSqlDialect::new();
        let statements = dialect.split_statements(sql);
        if statements.is_empty() {
            return Ok(vec![QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: None,
                message: "Nothing to run.".to_string(),
                is_error: false,
                elapsed: Duration::ZERO,
                database: Some(active_db),
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: None,
            }]);
        }

        let mut results = Vec::new();

        for span in statements {
            let db = client.database(&active_db);
            let stmt_text = span.text;

            match classify_statement(&stmt_text) {
                StatementKind::Shell => {
                    let fut = execute_shell_statement(&db, &stmt_text, max_rows, read_only);
                    let res = match timeout {
                        Some(limit) => match tokio::time::timeout(limit, fut).await {
                            Ok(r) => r,
                            Err(_) => {
                                let mut r = QueryResult::error(
                                    format!(
                                        "MongoDB command timed out after {} second(s).",
                                        limit.as_secs()
                                    ),
                                    limit,
                                    Some(active_db.clone()),
                                );
                                r.statement = Some(stmt_text);
                                r
                            }
                        },
                        None => fut.await,
                    };
                    let is_err = res.is_error;
                    results.push(res);
                    if is_err {
                        break;
                    }
                }
                StatementKind::Sql => {
                    let start = Instant::now();
                    let field_types = TSqlMongoCompiler::find_primary_collection(&stmt_text)
                        .map(|c| self.cached_field_types(&active_db, &c))
                        .unwrap_or_default();

                    let cmd = match TSqlMongoCompiler::compile_with_field_types(
                        &stmt_text,
                        &field_types,
                    ) {
                        Ok(c) => c,
                        Err(err) => {
                            let mut r = QueryResult::error(
                                err.to_string(),
                                start.elapsed(),
                                Some(active_db.clone()),
                            );
                            r.statement = Some(stmt_text);
                            results.push(r);
                            break;
                        }
                    };

                    if let MongoCommand::Use { database: next_db } = cmd {
                        active_db = next_db.clone();
                        results.push(QueryResult {
                            columns: Vec::new(),
                            rows: Vec::new(),
                            records_affected: None,
                            message: format!("Switched database to '{next_db}'."),
                            is_error: false,
                            elapsed: start.elapsed(),
                            database: Some(active_db.clone()),
                            is_truncated: false,
                            row_limit: None,
                            messages: Vec::new(),
                            statement: Some(stmt_text),
                        });
                        continue;
                    }

                    if read_only {
                        let allowed = match &cmd {
                            MongoCommand::Select { pipeline, .. } => {
                                !pipeline_has_write_stage(pipeline)
                            }
                            _ => false,
                        };
                        if !allowed {
                            let mut r = QueryResult::error(
                                "Statement refused: this workspace is in read-only mode.",
                                start.elapsed(),
                                Some(active_db.clone()),
                            );
                            r.statement = Some(stmt_text);
                            results.push(r);
                            break;
                        }
                    }

                    let fut = execute_sql_command(&db, cmd, &stmt_text, max_rows, start);
                    let res = match timeout {
                        Some(limit) => match tokio::time::timeout(limit, fut).await {
                            Ok(Ok(r)) => r,
                            Ok(Err(err)) => {
                                let mut r = QueryResult::error(
                                    err.to_string(),
                                    start.elapsed(),
                                    Some(active_db.clone()),
                                );
                                r.statement = Some(stmt_text);
                                r
                            }
                            Err(_) => {
                                let mut r = QueryResult::error(
                                    format!(
                                        "MongoDB query timed out after {} second(s).",
                                        limit.as_secs()
                                    ),
                                    start.elapsed(),
                                    Some(active_db.clone()),
                                );
                                r.statement = Some(stmt_text);
                                r
                            }
                        },
                        None => match fut.await {
                            Ok(r) => r,
                            Err(err) => {
                                let mut r = QueryResult::error(
                                    err.to_string(),
                                    start.elapsed(),
                                    Some(active_db.clone()),
                                );
                                r.statement = Some(stmt_text);
                                r
                            }
                        },
                    };

                    let is_err = res.is_error;
                    results.push(res);
                    if is_err {
                        break;
                    }
                }
            }
        }

        Ok(results)
    }
}

async fn execute_sql_command(
    db: &mongodb::Database,
    cmd: MongoCommand,
    raw_stmt: &str,
    max_rows: usize,
    start: Instant,
) -> Result<QueryResult> {
    let db_name = Some(db.name().to_string());
    match cmd {
        MongoCommand::Select {
            collection,
            mut pipeline,
            explicit_columns,
            limit,
        } => {
            let needs_cap = match limit {
                Some(l) if (l as usize) <= max_rows => false,
                _ => true,
            };
            if needs_cap {
                pipeline.push(doc! { "$limit": (max_rows as i64).saturating_add(1) });
            }

            let coll = db.collection::<Document>(&collection);
            let mut cursor = coll
                .aggregate(pipeline)
                .await
                .with_context(|| format!("Query on collection '{collection}' failed"))?;
            let mut docs = Vec::new();
            let mut truncated = false;
            while let Some(doc) = cursor
                .try_next()
                .await
                .with_context(|| format!("Reading cursor for '{collection}' failed"))?
            {
                if docs.len() < max_rows {
                    docs.push(doc);
                } else {
                    truncated = true;
                    break;
                }
            }

            Ok(documents_to_query_result(
                &docs,
                &explicit_columns,
                max_rows,
                truncated,
                start.elapsed(),
                db_name,
                Some(raw_stmt.to_string()),
            ))
        }
        MongoCommand::Insert {
            collection,
            documents,
        } => {
            let coll = db.collection::<Document>(&collection);
            let res = coll
                .insert_many(documents)
                .await
                .with_context(|| format!("INSERT into '{collection}' failed"))?;
            let count = res.inserted_ids.len() as i64;
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: Some(count),
                message: format!("{count} document(s) inserted."),
                is_error: false,
                elapsed: start.elapsed(),
                database: db_name,
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: Some(raw_stmt.to_string()),
            })
        }
        MongoCommand::Update {
            collection,
            filter,
            set_fields,
            unset_fields,
        } => {
            let mut update_doc = Document::new();
            if !set_fields.is_empty() {
                update_doc.insert("$set", set_fields);
            }
            if !unset_fields.is_empty() {
                update_doc.insert("$unset", unset_fields);
            }
            let coll = db.collection::<Document>(&collection);
            let res = coll
                .update_many(filter, update_doc)
                .await
                .with_context(|| format!("UPDATE on '{collection}' failed"))?;
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: Some(res.modified_count as i64),
                message: format!(
                    "Matched {} document(s), modified {} document(s).",
                    res.matched_count, res.modified_count
                ),
                is_error: false,
                elapsed: start.elapsed(),
                database: db_name,
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: Some(raw_stmt.to_string()),
            })
        }
        MongoCommand::Delete {
            collection,
            filter,
            has_where,
        } => {
            if !has_where {
                bail!(
                    "Refusing to DELETE all documents from '{collection}' without a WHERE clause. Add a WHERE clause or run db.{collection}.deleteMany({{}})."
                );
            }
            let coll = db.collection::<Document>(&collection);
            let res = coll
                .delete_many(filter)
                .await
                .with_context(|| format!("DELETE from '{collection}' failed"))?;
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: Some(res.deleted_count as i64),
                message: format!("{} document(s) deleted.", res.deleted_count),
                is_error: false,
                elapsed: start.elapsed(),
                database: db_name,
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: Some(raw_stmt.to_string()),
            })
        }
        MongoCommand::Use { database } => Ok(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            records_affected: None,
            message: format!("Switched database to '{database}'."),
            is_error: false,
            elapsed: start.elapsed(),
            database: Some(database),
            is_truncated: false,
            row_limit: None,
            messages: Vec::new(),
            statement: Some(raw_stmt.to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_metadata_settings_and_secret_keys_match_reference() {
        let provider = MongoProvider;
        assert_eq!(provider.id(), "mongodb");
        assert_eq!(provider.display_name(), "MongoDB");
        assert_eq!(provider.dialect(), "mongodb");
        assert!(provider.read_only_policy().is_some());
        assert_eq!(provider.password_keys(), vec!["connectionString", "password"]);

        let keys: Vec<&str> = provider.settings().iter().map(|s| s.key).collect();
        assert_eq!(
            keys,
            vec![
                "connectionString",
                "host",
                "port",
                "database",
                "authSource",
                "user",
                "password",
                "directConnection",
                "connectTimeout",
                "queryTimeout"
            ]
        );
    }

    #[test]
    fn endpoint_and_identity_handle_srv_standard_uri_and_discrete_fields() {
        let provider = MongoProvider;

        let mut p1 = provider.default_profile();
        assert_eq!(
            provider.describe_endpoint(&p1).as_deref(),
            Some("MongoDB · localhost:27017")
        );

        p1.set(
            "connectionString",
            "mongodb+srv://alice:supersecret@cluster0.abcde.mongodb.net/shop?retryWrites=true",
        );
        let ep = provider.describe_endpoint(&p1).unwrap();
        assert_eq!(ep, "MongoDB · cluster0.abcde.mongodb.net");
        assert!(!ep.contains("supersecret"));
        assert_eq!(provider.describe_identity(&p1).as_deref(), Some("alice"));

        let mut p2 = provider.default_profile();
        p2.set("connectionString", "mongodb://bob:secret@db.internal:27018/analytics");
        assert_eq!(
            provider.describe_endpoint(&p2).as_deref(),
            Some("MongoDB · db.internal:27018")
        );
        assert_eq!(provider.describe_identity(&p2).as_deref(), Some("bob"));
    }

    #[test]
    fn host_field_rejects_uri_without_echoing_secret() {
        let mut p = ConnectionProfile::new("mongodb");
        p.set("host", "mongodb+srv://admin:topsecret@cluster.example.net/db");
        let err = parse_host_and_port(&p).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Connection string"));
        assert!(!msg.contains("topsecret"));
    }

    #[test]
    fn uri_summary_extracts_default_database() {
        let s = parse_mongo_uri_summary("mongodb+srv://u:p@cluster.mongodb.net/inventory?w=majority")
            .unwrap();
        assert!(s.is_srv);
        assert_eq!(s.host_display, "cluster.mongodb.net");
        assert_eq!(s.username.as_deref(), Some("u"));
        assert_eq!(s.database.as_deref(), Some("inventory"));
    }

    #[test]
    fn editor_language_formats_collection_preview_and_bracket_quotes() {
        let provider = MongoProvider;
        let lang = provider.editor_language();
        assert_eq!(lang.quote_identifier("order.items"), "[order.items]");
        assert_eq!(
            lang.preview_statement(None, "orders", 100),
            "SELECT * FROM [orders] LIMIT 100;"
        );
    }
}
