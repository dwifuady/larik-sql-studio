use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use directories::BaseDirs;
use larik_data::{ConnectionProfile, ProviderCatalog};
use rusqlite::{Connection, OptionalExtension, params};

use crate::mcp::{
    MCP_MAX_STATEMENT_CHARS, McpAccess, McpApprovalRecord, McpLogRecord, McpSpaceRecord,
    mcp_approval_status, truncate_chars,
};
use crate::notes::{NoteAnchor, NoteKind, NoteRecord};
use crate::run_log::{
    DEFAULT_RUN_LOG_RETENTION_DAYS, RUN_LOG_RETENTION_CHOICES, RunLogRecord, escape_like_pattern,
    normalize_retention_days, truncate_statement,
};
use crate::secrets::{SecretBackend, WindowsCredentialBackend, secret_target};
use crate::snippets::{
    SnippetRecord, builtin_catalog, builtin_to_record, normalize_dialects, sort_snippets,
};
use crate::{SpaceState, StudioState};

const RUN_LOG_RETENTION_META_KEY: &str = "run_log_retention_days";
const MCP_ENABLED_META_KEY: &str = "mcp_enabled";

/// SQLite persistence for Larik SQL Studio.
/// Stored under `%LOCALAPPDATA%\Larik SQL Studio\studio.db` (or `LARIK_STATE_DIR\studio.db` when set).
pub struct StudioStore {
    db_path: PathBuf,
    secrets: Arc<dyn SecretBackend>,
}

impl StudioStore {
    fn has_custom_state_dir() -> bool {
        std::env::var("LARIK_STATE_DIR")
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
    }

    pub fn default_state_dir() -> PathBuf {
        if let Ok(custom) = std::env::var("LARIK_STATE_DIR") {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        let base = BaseDirs::new()
            .map(|b| b.data_local_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("Larik SQL Studio")
    }

    pub fn open_default() -> anyhow::Result<Self> {
        let dir = Self::default_state_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create data directory {}", dir.display()))?;
        let db_path = dir.join("studio.db");
        let secrets: Arc<dyn SecretBackend> = Arc::new(WindowsCredentialBackend);

        // When using the default app data directory (not an explicit `LARIK_STATE_DIR`),
        // also check the legacy Tauri `%APPDATA%\larik\larik-sql-studio\data\larik.db` location.
        let mut legacy_candidates = crate::migration::sibling_legacy_tauri_db_candidates(&db_path);
        if !Self::has_custom_state_dir() {
            legacy_candidates.extend(crate::migration::default_legacy_tauri_db_candidates());
        }
        Self::open_at_with_secrets_and_legacy(db_path, secrets, &legacy_candidates)
    }

    pub fn open_at(db_path: PathBuf) -> anyhow::Result<Self> {
        Self::open_at_with_secrets(db_path, Arc::new(WindowsCredentialBackend))
    }

    pub fn open_at_with_secrets(
        db_path: PathBuf,
        secrets: Arc<dyn SecretBackend>,
    ) -> anyhow::Result<Self> {
        let siblings = crate::migration::sibling_legacy_tauri_db_candidates(&db_path);
        Self::open_at_with_secrets_and_legacy(db_path, secrets, &siblings)
    }

    pub fn open_at_with_secrets_and_legacy(
        db_path: PathBuf,
        secrets: Arc<dyn SecretBackend>,
        legacy_candidates: &[PathBuf],
    ) -> anyhow::Result<Self> {
        let staged_in_place = crate::migration::prepare_in_place_migration(&db_path)?;

        let store = Self { db_path, secrets };
        store.ensure_schema()?;

        if let Some(staged) = staged_in_place {
            crate::migration::finish_in_place_migration(&store.db_path, &store.secrets, staged)?;
        } else {
            let _ = crate::migration::migrate_from_external_candidates(
                &store.db_path,
                &store.secrets,
                legacy_candidates,
            )?;
        }

        Ok(store)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn connect(&self) -> anyhow::Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(conn)
    }

    fn ensure_schema(&self) -> anyhow::Result<()> {
        let conn = self.connect()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS studio_snapshot (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                version INTEGER NOT NULL,
                payload TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS studio_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS snippets (
                id TEXT PRIMARY KEY,
                trigger TEXT NOT NULL,
                name TEXT NOT NULL,
                content TEXT NOT NULL,
                description TEXT,
                is_builtin INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1,
                category TEXT,
                drivers TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_snippets_trigger ON snippets(trigger);
            CREATE INDEX IF NOT EXISTS idx_snippets_enabled ON snippets(enabled);

            CREATE TABLE IF NOT EXISTS run_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                recorded_at TEXT NOT NULL DEFAULT (datetime('now')),
                actor TEXT NOT NULL,
                space_id TEXT,
                space_name TEXT,
                database_name TEXT,
                provider_id TEXT,
                tab_id TEXT,
                statement TEXT NOT NULL,
                status TEXT NOT NULL,
                reason TEXT,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                rows_returned INTEGER NOT NULL DEFAULT 0,
                row_limit INTEGER,
                is_truncated INTEGER NOT NULL DEFAULT 0,
                description TEXT,
                reference TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_run_log_recorded ON run_log(recorded_at DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_run_log_space ON run_log(space_id, recorded_at DESC, id DESC);

            CREATE TABLE IF NOT EXISTS space_mcp_access (
                space_id TEXT PRIMARY KEY,
                access TEXT NOT NULL DEFAULT 'off'
            );

            CREATE TABLE IF NOT EXISTS mcp_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                recorded_at TEXT NOT NULL DEFAULT (datetime('now')),
                tool TEXT NOT NULL,
                space_id TEXT,
                space_name TEXT,
                provider_id TEXT,
                access_level TEXT NOT NULL,
                statement TEXT,
                status TEXT NOT NULL,
                reason TEXT,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                rows_returned INTEGER NOT NULL DEFAULT 0,
                decision_surface TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_mcp_log_recorded ON mcp_log(recorded_at DESC, id DESC);

            CREATE TABLE IF NOT EXISTS mcp_approvals (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                space_id TEXT NOT NULL,
                space_name TEXT,
                statement TEXT NOT NULL,
                status TEXT NOT NULL,
                decided_at TEXT,
                surface TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_mcp_approvals_status ON mcp_approvals(status, created_at);

            CREATE TABLE IF NOT EXISTS notes (
                id            TEXT PRIMARY KEY,
                space_id      TEXT NOT NULL DEFAULT '',
                tab_id        TEXT NOT NULL,
                kind          TEXT NOT NULL DEFAULT 'note',
                body          TEXT NOT NULL DEFAULT '',
                anchor_start  INTEGER NOT NULL DEFAULT 0,
                anchor_length INTEGER NOT NULL DEFAULT 0,
                quote         TEXT NOT NULL DEFAULT '',
                prefix        TEXT NOT NULL DEFAULT '',
                suffix        TEXT NOT NULL DEFAULT '',
                line_hint     INTEGER NOT NULL DEFAULT 1,
                is_unanchored INTEGER NOT NULL DEFAULT 0,
                is_tab        INTEGER NOT NULL DEFAULT 0,
                created_at    TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_notes_tab ON notes(tab_id, anchor_start);
            CREATE INDEX IF NOT EXISTS idx_notes_space ON notes(space_id);",
        )?;
        Ok(())
    }

    pub fn load(&self, catalog: &ProviderCatalog) -> StudioState {
        let mut state = self.load_snapshot(catalog);
        for space in &mut state.spaces {
            if let Some(provider) = catalog.find(&space.profile.provider_id) {
                for key in provider.password_keys() {
                    let target = secret_target(&space.id, key);
                    if let Ok(Some(secret)) = self.secrets.get(&target) {
                        if !secret.is_empty() {
                            space.profile.set(key, secret);
                        }
                    }
                }
            }
        }
        state
    }

    pub fn load_window_placement(&self) -> Option<crate::WindowPlacementState> {
        #[derive(serde::Deserialize)]
        struct PlacementEnvelope {
            #[serde(default)]
            window_placement: Option<crate::WindowPlacementState>,
        }

        let conn = self.connect().ok()?;
        let payload: String = conn
            .query_row(
                "SELECT payload FROM studio_snapshot WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()?;
        serde_json::from_str::<PlacementEnvelope>(&payload)
            .ok()
            .and_then(|env| env.window_placement)
    }

    fn load_snapshot(&self, catalog: &ProviderCatalog) -> StudioState {
        let Ok(conn) = self.connect() else {
            return StudioState::seed_default(catalog);
        };
        let payload: Option<String> = conn
            .query_row(
                "SELECT payload FROM studio_snapshot WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();

        payload
            .and_then(|json| serde_json::from_str::<StudioState>(&json).ok())
            .filter(|state| !state.spaces.is_empty())
            .unwrap_or_else(|| StudioState::seed_default(catalog))
    }

    pub fn save(&self, state: &StudioState, catalog: &ProviderCatalog) -> anyhow::Result<()> {
        self.save_inner(state, catalog, true)
    }

    /// Persist the sanitized studio JSON snapshot without touching the OS secret backend.
    /// Used for frequent UI state changes (tab switches, editor edits, layout toggles) where
    /// workspace passwords have not changed, avoiding slow Windows Credential Manager IPC.
    pub fn save_snapshot_only(
        &self,
        state: &StudioState,
        catalog: &ProviderCatalog,
    ) -> anyhow::Result<()> {
        self.save_inner(state, catalog, false)
    }

    fn save_inner(
        &self,
        state: &StudioState,
        catalog: &ProviderCatalog,
        sync_secrets: bool,
    ) -> anyhow::Result<()> {
        // Always strip provider-declared password fields from SQLite JSON; only write them to
        // the OS secret backend when `sync_secrets` is requested.
        let mut sanitized = state.clone();
        for space in &mut sanitized.spaces {
            if let Some(provider) = catalog.find(&space.profile.provider_id) {
                for key in provider.password_keys() {
                    if sync_secrets {
                        let target = secret_target(&space.id, key);
                        match space.profile.get(key) {
                            Some(val) if !val.is_empty() => {
                                self.secrets.set(&target, val)?;
                            }
                            _ => {
                                self.secrets.delete(&target)?;
                            }
                        }
                    }
                    space
                        .profile
                        .values
                        .retain(|k, _| !k.eq_ignore_ascii_case(key));
                }
            }
        }

        let json = serde_json::to_string(&sanitized)?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO studio_snapshot (id, version, payload, updated_at)
             VALUES (1, 1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
            params![json, now],
        )?;
        // Prune notes whose tab is neither open nor archived (matching WinUI StudioStore.Write).
        let mut valid_tabs: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for space in &sanitized.spaces {
            for tab in &space.tabs {
                valid_tabs.insert(tab.id.as_str());
            }
        }
        for arch in &sanitized.archived_tabs {
            valid_tabs.insert(arch.id.as_str());
        }
        let existing_note_tabs: Vec<String> = {
            let mut stmt = tx.prepare("SELECT DISTINCT tab_id FROM notes")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.filter_map(Result::ok).collect()
        };
        if !existing_note_tabs.is_empty() {
            let mut del = tx.prepare("DELETE FROM notes WHERE tab_id = ?1")?;
            for tab_id in existing_note_tabs {
                if !valid_tabs.contains(tab_id.as_str()) {
                    del.execute(params![tab_id])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_space_secrets(
        &self,
        space: &SpaceState,
        catalog: &ProviderCatalog,
    ) -> anyhow::Result<()> {
        if let Some(provider) = catalog.find(&space.profile.provider_id) {
            for key in provider.password_keys() {
                let target = secret_target(&space.id, key);
                self.secrets.delete(&target)?;
            }
        }
        Ok(())
    }

    pub fn load_snippets(&self) -> anyhow::Result<Vec<SnippetRecord>> {
        let mut conn = self.connect()?;
        {
            let tx = conn.transaction()?;
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO snippets
                    (id, trigger, name, content, description, is_builtin, enabled, category, drivers)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, ?6, ?7)",
            )?;
            for def in builtin_catalog() {
                let rec = builtin_to_record(&def, true);
                let drivers = drivers_column(rec.dialects.as_deref());
                stmt.execute(params![
                    rec.id,
                    rec.trigger,
                    rec.name,
                    rec.content,
                    rec.description,
                    rec.category,
                    drivers,
                ])?;
            }
            drop(stmt);
            tx.commit()?;
        }

        let mut stmt = conn.prepare(
            "SELECT id, trigger, name, content, description, is_builtin, enabled, category, drivers
             FROM snippets",
        )?;
        let rows = stmt.query_map([], |row| {
            let is_builtin: i64 = row.get(5)?;
            let is_enabled: i64 = row.get(6)?;
            let drivers: Option<String> = row.get(8)?;
            Ok(SnippetRecord {
                id: row.get(0)?,
                trigger: row.get(1)?,
                name: row.get(2)?,
                content: row.get(3)?,
                description: row.get(4)?,
                category: row.get(7)?,
                dialects: parse_drivers(drivers.as_deref()),
                is_builtin: is_builtin != 0,
                is_enabled: is_enabled != 0,
            })
        })?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        sort_snippets(&mut records);
        Ok(records)
    }

    pub fn save_snippet(&self, record: &SnippetRecord) -> anyhow::Result<()> {
        let conn = self.connect()?;
        let normalized_dialects = normalize_dialects(record.dialects.clone());
        let drivers = drivers_column(normalized_dialects.as_deref());
        let description = record
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let category = record
            .category
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        conn.execute(
            "INSERT INTO snippets
                (id, trigger, name, content, description, is_builtin, enabled, category, drivers)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                trigger = excluded.trigger,
                name = excluded.name,
                content = excluded.content,
                description = excluded.description,
                enabled = excluded.enabled,
                category = excluded.category,
                drivers = excluded.drivers,
                updated_at = datetime('now')",
            params![
                record.id,
                record.trigger.trim(),
                record.name.trim(),
                record.content,
                description,
                if record.is_builtin { 1 } else { 0 },
                if record.is_enabled { 1 } else { 0 },
                category,
                drivers,
            ],
        )?;
        Ok(())
    }

    pub fn delete_snippet(&self, id: &str) -> anyhow::Result<bool> {
        let conn = self.connect()?;
        let deleted = conn.execute(
            "DELETE FROM snippets WHERE id = ?1 AND is_builtin = 0",
            params![id],
        )?;
        Ok(deleted > 0)
    }

    // ---------------------------------------------------------------- run_log

    /// Records a query execution in `run_log`. Best-effort by contract: never fails query execution.
    pub fn log_run(&self, record: &RunLogRecord) -> bool {
        let Ok(conn) = self.connect() else {
            return false;
        };
        let statement = truncate_statement(&record.statement);
        conn.execute(
            "INSERT INTO run_log (
                actor, space_id, space_name, database_name, provider_id, tab_id,
                statement, status, reason, duration_ms, rows_returned, row_limit,
                is_truncated, description, reference
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15
             )",
            params![
                record.actor,
                record.space_id,
                record.space_name,
                record.database_name,
                record.provider_id,
                record.tab_id,
                statement,
                record.status,
                record.reason,
                record.duration_ms.max(0),
                record.rows_returned.max(0),
                record.row_limit,
                if record.is_truncated { 1i64 } else { 0i64 },
                record.description,
                record.reference,
            ],
        )
        .is_ok()
    }

    /// Reads the most recent run-log rows, optionally scoped to a workspace.
    pub fn get_run_log(&self, space_id: Option<&str>, limit: usize) -> Vec<RunLogRecord> {
        let Ok(conn) = self.connect() else {
            return Vec::new();
        };
        let limit = limit.clamp(1, 5_000) as i64;
        let sql = match space_id {
            Some(_) => {
                "SELECT id, recorded_at, actor, space_id, space_name, database_name, provider_id,
                        tab_id, statement, status, reason, duration_ms, rows_returned, row_limit,
                        is_truncated, description, reference
                 FROM run_log
                 WHERE space_id = ?1
                 ORDER BY recorded_at DESC, id DESC
                 LIMIT ?2"
            }
            None => {
                "SELECT id, recorded_at, actor, space_id, space_name, database_name, provider_id,
                        tab_id, statement, status, reason, duration_ms, rows_returned, row_limit,
                        is_truncated, description, reference
                 FROM run_log
                 ORDER BY recorded_at DESC, id DESC
                 LIMIT ?1"
            }
        };

        let Ok(mut stmt) = conn.prepare(sql) else {
            return Vec::new();
        };
        let mapped = match space_id {
            Some(sid) => stmt.query_map(params![sid, limit], map_run_log_row),
            None => stmt.query_map(params![limit], map_run_log_row),
        };
        match mapped {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Counts all persisted run-log rows in the given scope (or across all workspaces when `None`).
    pub fn count_run_log(&self, space_id: Option<&str>) -> usize {
        let Ok(conn) = self.connect() else {
            return 0;
        };
        let count: rusqlite::Result<i64> = match space_id {
            Some(sid) => conn.query_row(
                "SELECT COUNT(*) FROM run_log WHERE space_id = ?1",
                params![sid],
                |r| r.get(0),
            ),
            None => conn.query_row("SELECT COUNT(*) FROM run_log", [], |r| r.get(0)),
        };
        count.unwrap_or(0).max(0) as usize
    }

    /// Substring search over statement, space_name, database_name, description, and reference.
    pub fn search_run_log(
        &self,
        space_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Vec<RunLogRecord> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return self.get_run_log(space_id, limit);
        }
        let Ok(conn) = self.connect() else {
            return Vec::new();
        };
        let pattern = escape_like_pattern(trimmed);
        let limit = limit.clamp(1, 5_000) as i64;
        let sql = match space_id {
            Some(_) => {
                "SELECT id, recorded_at, actor, space_id, space_name, database_name, provider_id,
                        tab_id, statement, status, reason, duration_ms, rows_returned, row_limit,
                        is_truncated, description, reference
                 FROM run_log
                 WHERE space_id = ?1
                   AND (statement LIKE ?2 ESCAPE '\\'
                        OR COALESCE(space_name, '') LIKE ?2 ESCAPE '\\'
                        OR COALESCE(database_name, '') LIKE ?2 ESCAPE '\\'
                        OR COALESCE(description, '') LIKE ?2 ESCAPE '\\'
                        OR COALESCE(reference, '') LIKE ?2 ESCAPE '\\')
                 ORDER BY recorded_at DESC, id DESC
                 LIMIT ?3"
            }
            None => {
                "SELECT id, recorded_at, actor, space_id, space_name, database_name, provider_id,
                        tab_id, statement, status, reason, duration_ms, rows_returned, row_limit,
                        is_truncated, description, reference
                 FROM run_log
                 WHERE statement LIKE ?1 ESCAPE '\\'
                    OR COALESCE(space_name, '') LIKE ?1 ESCAPE '\\'
                    OR COALESCE(database_name, '') LIKE ?1 ESCAPE '\\'
                    OR COALESCE(description, '') LIKE ?1 ESCAPE '\\'
                    OR COALESCE(reference, '') LIKE ?1 ESCAPE '\\'
                 ORDER BY recorded_at DESC, id DESC
                 LIMIT ?2"
            }
        };

        let Ok(mut stmt) = conn.prepare(sql) else {
            return Vec::new();
        };
        let mapped = match space_id {
            Some(sid) => stmt.query_map(params![sid, pattern, limit], map_run_log_row),
            None => stmt.query_map(params![pattern, limit], map_run_log_row),
        };
        match mapped {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn get_run_log_retention_days(&self) -> i32 {
        let Ok(conn) = self.connect() else {
            return DEFAULT_RUN_LOG_RETENTION_DAYS;
        };
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM studio_meta WHERE key = ?1",
                params![RUN_LOG_RETENTION_META_KEY],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        raw.and_then(|s| s.parse::<i32>().ok())
            .map(normalize_retention_days)
            .unwrap_or(DEFAULT_RUN_LOG_RETENTION_DAYS)
    }

    pub fn set_run_log_retention_days(&self, days: i32) -> bool {
        if !RUN_LOG_RETENTION_CHOICES.contains(&days) {
            return false;
        }
        let Ok(conn) = self.connect() else {
            return false;
        };
        conn.execute(
            "INSERT INTO studio_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![RUN_LOG_RETENTION_META_KEY, days.to_string()],
        )
        .is_ok()
    }

    /// Removes run-log rows older than the configured retention window.
    pub fn prune_run_log(&self) -> usize {
        let Ok(conn) = self.connect() else {
            return 0;
        };
        let days = self.get_run_log_retention_days();
        let modifier = format!("-{days} days");
        conn.execute(
            "DELETE FROM run_log WHERE recorded_at < datetime('now', ?1)",
            params![modifier],
        )
        .unwrap_or(0)
    }

    // ------------------------------------------------------------------ MCP

    /// Global AI-agents opt-in flag (`studio_meta['mcp_enabled']`). Fails closed to `false`.
    pub fn is_mcp_enabled(&self) -> bool {
        let Ok(conn) = self.connect() else {
            return false;
        };
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM studio_meta WHERE key = ?1",
                params![MCP_ENABLED_META_KEY],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        matches!(
            raw.as_deref().map(str::trim),
            Some("true" | "1" | "TRUE" | "True")
        )
    }

    pub fn set_mcp_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO studio_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![MCP_ENABLED_META_KEY, if enabled { "true" } else { "false" }],
        )?;
        Ok(())
    }

    /// Reads one workspace's MCP access level. Fails closed to `McpAccess::Off`.
    pub fn get_space_mcp_access(&self, space_id: &str) -> McpAccess {
        let Ok(conn) = self.connect() else {
            return McpAccess::Off;
        };
        let raw: Option<String> = conn
            .query_row(
                "SELECT access FROM space_mcp_access WHERE space_id = ?1",
                params![space_id],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        McpAccess::parse_fail_closed(raw.as_deref())
    }

    pub fn set_space_mcp_access(&self, space_id: &str, access: McpAccess) -> anyhow::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO space_mcp_access (space_id, access) VALUES (?1, ?2)
             ON CONFLICT(space_id) DO UPDATE SET access = excluded.access",
            params![space_id, access.as_storage()],
        )?;
        Ok(())
    }

    /// Returns all saved workspaces paired with their MCP access levels (non-secret profiles only).
    /// Never reads OS Credential Manager secrets and never invents default spaces on an empty DB.
    pub fn get_mcp_spaces(&self) -> Vec<McpSpaceRecord> {
        let Ok(conn) = self.connect() else {
            return Vec::new();
        };
        let payload: Option<String> = conn
            .query_row(
                "SELECT payload FROM studio_snapshot WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let Some(state) = payload.and_then(|json| serde_json::from_str::<StudioState>(&json).ok())
        else {
            return Vec::new();
        };

        let mut access_map: HashMap<String, McpAccess> = HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT space_id, access FROM space_mcp_access") {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for (sid, raw) in rows.flatten() {
                    access_map.insert(sid, McpAccess::parse_fail_closed(Some(&raw)));
                }
            }
        }

        state
            .spaces
            .into_iter()
            .map(|space| {
                let access = access_map
                    .get(&space.id)
                    .copied()
                    .unwrap_or(McpAccess::Off);
                let has_connection = !space.profile.values.is_empty();
                McpSpaceRecord {
                    id: space.id,
                    name: space.name,
                    provider_id: space.profile.provider_id.clone(),
                    access,
                    has_connection,
                    visible_profile: space.profile,
                }
            })
            .collect()
    }

    /// Hydrates a workspace's connection profile with passwords from the OS secret backend.
    /// Used only inside `larik-mcp` immediately before opening a live database connection.
    pub fn load_space_profile_with_secrets(
        &self,
        space: &McpSpaceRecord,
        catalog: &ProviderCatalog,
    ) -> ConnectionProfile {
        let mut profile = space.visible_profile.clone();
        if let Some(provider) = catalog.find(&space.provider_id) {
            for key in provider.password_keys() {
                let target = secret_target(&space.id, key);
                if let Ok(Some(secret)) = self.secrets.get(&target) {
                    if !secret.is_empty() {
                        profile.set(key, secret);
                    }
                }
            }
        }
        profile
    }

    /// Appends one entry to `mcp_log`. Best-effort; returns `true` on success.
    pub fn log_mcp_call(&self, entry: &McpLogRecord) -> bool {
        let Ok(conn) = self.connect() else {
            return false;
        };
        let statement = entry
            .statement
            .as_deref()
            .map(|s| truncate_chars(s, MCP_MAX_STATEMENT_CHARS));
        conn.execute(
            "INSERT INTO mcp_log (
                tool, space_id, space_name, provider_id, access_level,
                statement, status, reason, duration_ms, rows_returned, decision_surface
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                entry.tool,
                entry.space_id,
                entry.space_name,
                entry.provider_id,
                entry.access_level,
                statement,
                entry.status,
                entry.reason,
                entry.duration_ms.max(0),
                entry.rows_returned.max(0),
                entry.decision_surface,
            ],
        )
        .is_ok()
    }

    /// Reads the newest `limit` entries from `mcp_log`.
    pub fn get_mcp_log(&self, limit: usize) -> Vec<McpLogRecord> {
        let Ok(conn) = self.connect() else {
            return Vec::new();
        };
        let limit = limit.clamp(1, 500) as i64;
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, recorded_at, tool, space_id, space_name, provider_id, access_level,
                    statement, status, reason, duration_ms, rows_returned, decision_surface
             FROM mcp_log
             ORDER BY recorded_at DESC, id DESC
             LIMIT ?1",
        ) else {
            return Vec::new();
        };
        let mapped = stmt.query_map(params![limit], |row| {
            Ok(McpLogRecord {
                id: row.get(0)?,
                recorded_at: row.get(1)?,
                tool: row.get(2)?,
                space_id: row.get(3)?,
                space_name: row.get(4)?,
                provider_id: row.get(5)?,
                access_level: row.get(6)?,
                statement: row.get(7)?,
                status: row.get(8)?,
                reason: row.get(9)?,
                duration_ms: row.get(10)?,
                rows_returned: row.get(11)?,
                decision_surface: row.get(12)?,
            })
        });
        match mapped {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn clear_mcp_log(&self) -> anyhow::Result<()> {
        let conn = self.connect()?;
        conn.execute("DELETE FROM mcp_log", [])?;
        Ok(())
    }

    pub fn create_mcp_approval(
        &self,
        id: &str,
        space_id: &str,
        space_name: Option<&str>,
        statement: &str,
    ) -> anyhow::Result<()> {
        let conn = self.connect()?;
        let stmt = truncate_chars(statement, MCP_MAX_STATEMENT_CHARS);
        conn.execute(
            "INSERT INTO mcp_approvals (id, space_id, space_name, statement, status)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, space_id, space_name, stmt, mcp_approval_status::PENDING],
        )?;
        Ok(())
    }

    pub fn resolve_mcp_approval(
        &self,
        id: &str,
        status: &str,
        surface: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "UPDATE mcp_approvals
             SET status = ?2, decided_at = datetime('now'), surface = ?3
             WHERE id = ?1",
            params![id, status, surface],
        )?;
        Ok(())
    }

    /// Marks stale `pending` approvals older than 10 minutes as `orphaned` (called on studio startup).
    pub fn sweep_mcp_approvals(&self) -> usize {
        let Ok(conn) = self.connect() else {
            return 0;
        };
        conn.execute(
            "UPDATE mcp_approvals
             SET status = ?1, decided_at = datetime('now')
             WHERE status = ?2 AND created_at < datetime('now', '-10 minutes')",
            params![mcp_approval_status::ORPHANED, mcp_approval_status::PENDING],
        )
        .unwrap_or(0)
    }

    pub fn get_mcp_approval(&self, id: &str) -> Option<McpApprovalRecord> {
        let Ok(conn) = self.connect() else {
            return None;
        };
        conn.query_row(
            "SELECT id, created_at, space_id, space_name, statement, status, decided_at, surface
             FROM mcp_approvals WHERE id = ?1",
            params![id],
            |row| {
                Ok(McpApprovalRecord {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    space_id: row.get(2)?,
                    space_name: row.get(3)?,
                    statement: row.get(4)?,
                    status: row.get(5)?,
                    decided_at: row.get(6)?,
                    surface: row.get(7)?,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    // =========================================================================
    // Notes persistence (ported from WinUI Larik.Core/NoteStore.cs)
    // =========================================================================

    pub fn get_notes_for_tab(&self, tab_id: &str) -> anyhow::Result<Vec<NoteRecord>> {
        if tab_id.trim().is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, space_id, tab_id, kind, body, anchor_start, anchor_length,
                    quote, prefix, suffix, line_hint, is_unanchored, is_tab,
                    created_at, updated_at
             FROM notes
             WHERE tab_id = ?1
             ORDER BY is_unanchored ASC, anchor_start ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![tab_id], map_note_row)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn get_notes_for_space(&self, space_id: &str) -> anyhow::Result<Vec<NoteRecord>> {
        if space_id.trim().is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, space_id, tab_id, kind, body, anchor_start, anchor_length,
                    quote, prefix, suffix, line_hint, is_unanchored, is_tab,
                    created_at, updated_at
             FROM notes
             WHERE space_id = ?1
             ORDER BY is_unanchored ASC, anchor_start ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![space_id], map_note_row)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn get_all_notes(&self) -> anyhow::Result<Vec<NoteRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, space_id, tab_id, kind, body, anchor_start, anchor_length,
                    quote, prefix, suffix, line_hint, is_unanchored, is_tab,
                    created_at, updated_at
             FROM notes
             ORDER BY is_unanchored ASC, anchor_start ASC, created_at ASC",
        )?;
        let rows = stmt.query_map([], map_note_row)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn save_note(&self, note: &NoteRecord) -> anyhow::Result<()> {
        let conn = self.connect()?;
        upsert_note(&conn, note)
    }

    pub fn save_notes_batch(&self, notes: &[NoteRecord]) -> anyhow::Result<()> {
        if notes.is_empty() {
            return Ok(());
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        for note in notes {
            upsert_note(&tx, note)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_note(&self, note_id: &str) -> anyhow::Result<()> {
        if note_id.trim().is_empty() {
            return Ok(());
        }
        let conn = self.connect()?;
        conn.execute("DELETE FROM notes WHERE id = ?1", params![note_id])?;
        Ok(())
    }

    pub fn delete_notes_for_tab(&self, tab_id: &str) -> anyhow::Result<()> {
        if tab_id.trim().is_empty() {
            return Ok(());
        }
        let conn = self.connect()?;
        conn.execute("DELETE FROM notes WHERE tab_id = ?1", params![tab_id])?;
        Ok(())
    }

    pub fn delete_notes_for_tabs(&self, tab_ids: &[String]) -> anyhow::Result<()> {
        if tab_ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare("DELETE FROM notes WHERE tab_id = ?1")?;
            for id in tab_ids {
                if !id.trim().is_empty() {
                    stmt.execute(params![id])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Updates `space_id` on all notes for `tab_id` (used when restoring an archived tab
    /// into a different workspace than its original deleted space).
    pub fn reassign_notes_space(&self, tab_id: &str, space_id: &str) -> anyhow::Result<()> {
        if tab_id.trim().is_empty() || space_id.trim().is_empty() {
            return Ok(());
        }
        let conn = self.connect()?;
        conn.execute(
            "UPDATE notes SET space_id = ?1 WHERE tab_id = ?2",
            params![space_id, tab_id],
        )?;
        Ok(())
    }
}

fn upsert_note(conn: &Connection, note: &NoteRecord) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO notes (
            id, space_id, tab_id, kind, body, anchor_start, anchor_length,
            quote, prefix, suffix, line_hint, is_unanchored, is_tab,
            created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
        ON CONFLICT(id) DO UPDATE SET
            space_id = excluded.space_id,
            tab_id = excluded.tab_id,
            kind = excluded.kind,
            body = excluded.body,
            anchor_start = excluded.anchor_start,
            anchor_length = excluded.anchor_length,
            quote = excluded.quote,
            prefix = excluded.prefix,
            suffix = excluded.suffix,
            line_hint = excluded.line_hint,
            is_unanchored = excluded.is_unanchored,
            is_tab = excluded.is_tab,
            updated_at = excluded.updated_at",
        params![
            note.id,
            note.space_id,
            note.tab_id,
            note.kind.as_str(),
            note.body,
            note.anchor.start as i64,
            note.anchor.length as i64,
            note.anchor.quote,
            note.anchor.prefix,
            note.anchor.suffix,
            note.anchor.line_hint as i64,
            if note.is_unanchored { 1i64 } else { 0i64 },
            if note.anchor.is_tab { 1i64 } else { 0i64 },
            note.created_at,
            note.updated_at,
        ],
    )?;
    Ok(())
}

fn map_note_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteRecord> {
    let kind_raw: String = row.get(3)?;
    let start: i64 = row.get(5)?;
    let length: i64 = row.get(6)?;
    let line_hint: i64 = row.get(10)?;
    let is_unanchored: i64 = row.get(11)?;
    let is_tab: i64 = row.get(12)?;
    Ok(NoteRecord {
        id: row.get(0)?,
        space_id: row.get(1)?,
        tab_id: row.get(2)?,
        kind: NoteKind::parse(&kind_raw),
        body: row.get(4)?,
        anchor: NoteAnchor {
            start: start.max(0) as usize,
            length: length.max(0) as usize,
            quote: row.get(7)?,
            prefix: row.get(8)?,
            suffix: row.get(9)?,
            line_hint: line_hint.max(0) as usize,
            is_tab: is_tab != 0,
        },
        is_unanchored: is_unanchored != 0,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn map_run_log_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunLogRecord> {
    let is_truncated: i64 = row.get(14)?;
    Ok(RunLogRecord {
        id: row.get(0)?,
        recorded_at: row.get(1)?,
        actor: row.get(2)?,
        space_id: row.get(3)?,
        space_name: row.get(4)?,
        database_name: row.get(5)?,
        provider_id: row.get(6)?,
        tab_id: row.get(7)?,
        statement: row.get(8)?,
        status: row.get(9)?,
        reason: row.get(10)?,
        duration_ms: row.get(11)?,
        rows_returned: row.get(12)?,
        row_limit: row.get(13)?,
        is_truncated: is_truncated != 0,
        description: row.get(15)?,
        reference: row.get(16)?,
    })
}

fn drivers_column(dialects: Option<&[String]>) -> Option<String> {
    dialects.filter(|d| !d.is_empty()).map(|d| d.join(","))
}

fn parse_drivers(value: Option<&str>) -> Option<Vec<String>> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    let parts: Vec<String> = raw
        .split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    normalize_dialects(Some(parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    use larik_data::sql_language::SqlLanguage;
    use larik_data::{
        ConnectionProfile, DatabaseConnection, DatabaseProvider, EditorLanguage, ProviderSetting,
        ProviderSettingKind,
    };

    use crate::secrets::InMemorySecretBackend;
    use crate::snippets::{has_trigger_conflict, snippets_for_dialect};

    struct TestSqlProvider;

    impl DatabaseProvider for TestSqlProvider {
        fn id(&self) -> &'static str {
            "test-sql"
        }

        fn display_name(&self) -> &'static str {
            "Test SQL"
        }

        fn dialect(&self) -> &'static str {
            "mssql"
        }

        fn settings(&self) -> &'static [ProviderSetting] {
            static SETTINGS: [ProviderSetting; 2] = [
                ProviderSetting {
                    key: "server",
                    label: "Server",
                    kind: ProviderSettingKind::Text,
                    required: true,
                    default_value: Some("localhost"),
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
            ];
            &SETTINGS
        }

        fn editor_language(&self) -> Arc<dyn EditorLanguage> {
            Arc::new(SqlLanguage::tsql())
        }

        fn describe_endpoint(&self, profile: &ConnectionProfile) -> Option<String> {
            profile.get("server").map(str::to_string)
        }

        fn create_connection(&self, _profile: ConnectionProfile) -> Box<dyn DatabaseConnection> {
            unimplemented!("not needed for store unit tests")
        }
    }

    #[test]
    fn round_trips_state_and_secrets_without_persisting_passwords_in_sqlite() {
        let dir = std::env::temp_dir().join(format!("larik_store_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("test.db");

        let mut catalog = ProviderCatalog::new();
        catalog.register(Arc::new(TestSqlProvider));

        let mut state = StudioState::seed_default(&catalog);
        state.spaces[0].name = "Custom Space".into();
        state.spaces[0].profile = catalog.find("test-sql").unwrap().default_profile();
        state.spaces[0].profile.set("password", "s3cr3t-p@ss-🔑");

        let secrets = Arc::new(InMemorySecretBackend::new());
        let store = StudioStore::open_at_with_secrets(db.clone(), secrets.clone()).unwrap();
        store.save(&state, &catalog).unwrap();

        // Password must NOT appear anywhere in SQLite snapshot payload.
        let conn = Connection::open(&db).unwrap();
        let raw_payload: String = conn
            .query_row("SELECT payload FROM studio_snapshot WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!raw_payload.contains("s3cr3t-p@ss-🔑"));

        // Password MUST be saved in the secret backend and rehydrated on load.
        assert_eq!(secrets.len(), 1);
        let loaded = store.load(&catalog);
        assert_eq!(loaded.spaces[0].name, "Custom Space");
        assert_eq!(
            loaded.spaces[0].profile.get("password"),
            Some("s3cr3t-p@ss-🔑")
        );

        // Clearing password deletes the secret from the backend.
        let mut cleared = loaded.clone();
        cleared.spaces[0].profile.set("password", "");
        store.save(&cleared, &catalog).unwrap();
        assert_eq!(secrets.len(), 0);
        assert_eq!(store.load(&catalog).spaces[0].profile.get("password"), None);

        // Deleting workspace secrets removes stored credentials.
        store.save(&loaded, &catalog).unwrap();
        assert_eq!(secrets.len(), 1);
        store
            .delete_space_secrets(&loaded.spaces[0], &catalog)
            .unwrap();
        assert_eq!(secrets.len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_snapshot_only_strips_passwords_without_touching_secret_backend() {
        let dir = std::env::temp_dir().join(format!("larik_store_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("test.db");

        let mut catalog = ProviderCatalog::new();
        catalog.register(Arc::new(TestSqlProvider));

        let mut state = StudioState::seed_default(&catalog);
        state.spaces[0].name = "Fast Tab Switch".into();
        state.spaces[0].profile = catalog.find("test-sql").unwrap().default_profile();
        state.spaces[0].profile.set("password", "never-in-sqlite");

        let secrets = Arc::new(InMemorySecretBackend::new());
        let store = StudioStore::open_at_with_secrets(db.clone(), secrets.clone()).unwrap();
        store.save_snapshot_only(&state, &catalog).unwrap();

        // Must NOT touch the secret backend.
        assert_eq!(secrets.len(), 0);

        // Must still strip passwords from SQLite payload while saving snapshot state.
        let conn = Connection::open(&db).unwrap();
        let raw_payload: String = conn
            .query_row("SELECT payload FROM studio_snapshot WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!raw_payload.contains("never-in-sqlite"));
        assert!(raw_payload.contains("Fast Tab Switch"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_credential_backend_round_trips_and_cleans_up() {
        let backend = WindowsCredentialBackend;
        let target = format!("larik-store-test-{}", uuid::Uuid::new_v4());
        let secret_value = "p@ssword_é_🔑_123!";

        assert_eq!(backend.get(&target).unwrap(), None);
        backend.set(&target, secret_value).unwrap();
        assert_eq!(backend.get(&target).unwrap().as_deref(), Some(secret_value));

        backend.delete(&target).unwrap();
        assert_eq!(backend.get(&target).unwrap(), None);
        // Deleting missing credential is idempotent.
        backend.delete(&target).unwrap();
    }

    #[test]
    fn snippets_seed_crud_conflict_and_dialect_filtering() {
        let dir = std::env::temp_dir().join(format!("larik_snip_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("snippets.db");

        let store = StudioStore::open_at(db).unwrap();

        // Seeding twice produces the exact 25 built-ins without duplicates.
        let first = store.load_snippets().unwrap();
        assert_eq!(first.len(), 25);
        let second = store.load_snippets().unwrap();
        assert_eq!(second.len(), 25);

        // Dialect filtering: mssql gets `selt` not `sell`; postgres gets `sell` not `selt`.
        let mssql_defs = snippets_for_dialect(&first, "mssql");
        assert!(mssql_defs.iter().any(|d| d.trigger == "selt"));
        assert!(!mssql_defs.iter().any(|d| d.trigger == "sell"));

        let pg_defs = snippets_for_dialect(&first, "postgres");
        assert!(pg_defs.iter().any(|d| d.trigger == "sell"));
        assert!(!pg_defs.iter().any(|d| d.trigger == "selt"));

        // Built-ins cannot be deleted.
        let builtin_sel = first.iter().find(|s| s.trigger == "sel").unwrap();
        assert!(!store.delete_snippet(&builtin_sel.id).unwrap());

        // Trigger conflict detection across overlapping scopes.
        assert!(has_trigger_conflict(&first, "SEL", None, None));
        assert!(has_trigger_conflict(
            &first,
            "selt",
            Some(&["mssql".to_string()]),
            None
        ));
        assert!(!has_trigger_conflict(
            &first,
            "selt",
            Some(&["postgres".to_string()]),
            None
        ));
        assert!(!has_trigger_conflict(
            &first,
            "sel",
            None,
            Some(&builtin_sel.id)
        ));

        // Custom snippet CRUD.
        let custom = SnippetRecord {
            id: uuid::Uuid::new_v4().to_string(),
            trigger: "mytop".to_string(),
            name: "My Top 10".to_string(),
            content: "SELECT TOP 10 * FROM ${cursor}".to_string(),
            description: Some("Quick top 10".to_string()),
            category: Some("Select".to_string()),
            dialects: Some(vec!["mssql".to_string()]),
            is_builtin: false,
            is_enabled: true,
        };
        store.save_snippet(&custom).unwrap();
        let after_create = store.load_snippets().unwrap();
        assert_eq!(after_create.len(), 26);
        assert!(
            snippets_for_dialect(&after_create, "mssql")
                .iter()
                .any(|d| d.trigger == "mytop")
        );
        assert!(
            !snippets_for_dialect(&after_create, "postgres")
                .iter()
                .any(|d| d.trigger == "mytop")
        );

        let mut updated = custom.clone();
        updated.is_enabled = false;
        store.save_snippet(&updated).unwrap();
        let after_disable = store.load_snippets().unwrap();
        assert!(
            !snippets_for_dialect(&after_disable, "mssql")
                .iter()
                .any(|d| d.trigger == "mytop")
        );

        assert!(store.delete_snippet(&custom.id).unwrap());
        let after_delete = store.load_snippets().unwrap();
        assert_eq!(after_delete.len(), 25);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_log_records_queries_searches_and_prunes_by_retention() {
        use crate::run_log::{RunLogRecord, run_status};

        let dir = std::env::temp_dir().join(format!("larik_runlog_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("test.db");

        let store =
            StudioStore::open_at_with_secrets(db.clone(), Arc::new(InMemorySecretBackend::new()))
                .unwrap();

        assert_eq!(store.get_run_log_retention_days(), 30);
        assert!(!store.set_run_log_retention_days(45));
        assert!(store.set_run_log_retention_days(90));
        assert_eq!(store.get_run_log_retention_days(), 90);

        assert!(store.log_run(&RunLogRecord::new_user_run(
            Some("space-1".into()),
            Some("Proxmox".into()),
            Some("blazex-hub".into()),
            Some("mssql".into()),
            Some("tab-1".into()),
            "SELECT * FROM dbo.customers WHERE name LIKE '50%'".into(),
            run_status::OK,
            None,
            42,
            12,
            None,
            false,
        )));

        assert!(store.log_run(&RunLogRecord::new_user_run(
            Some("space-2".into()),
            Some("Demo Studio".into()),
            Some("larik_prod".into()),
            Some("demo".into()),
            Some("tab-2".into()),
            "SELECT * FROM dbo.orders".into(),
            run_status::FAILED,
            Some("Invalid object name".into()),
            7,
            0,
            None,
            false,
        )));

        assert_eq!(store.count_run_log(None), 2);
        assert_eq!(store.count_run_log(Some("space-1")), 1);
        assert_eq!(store.get_run_log(Some("space-1"), 50).len(), 1);
        assert_eq!(store.get_run_log(None, 50).len(), 2);

        // Escaped '%' search must match literal "50%" and NOT match the orders query.
        let pct_hits = store.search_run_log(None, "50%", 50);
        assert_eq!(pct_hits.len(), 1);
        assert_eq!(pct_hits[0].space_id.as_deref(), Some("space-1"));

        // Backdate one record older than 90 days and verify pruning removes only that row.
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE run_log SET recorded_at = datetime('now', '-100 days') WHERE space_id = 'space-2'",
            [],
        )
        .unwrap();
        drop(conn);

        let pruned = store.prune_run_log();
        assert_eq!(pruned, 1);
        assert_eq!(store.count_run_log(None), 1);
        assert_eq!(store.get_run_log(None, 50)[0].space_id.as_deref(), Some("space-1"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_archive_skips_pinned_and_active_tabs_and_keeps_one_per_space() {
        let mut catalog = ProviderCatalog::new();
        catalog.register(Arc::new(TestSqlProvider));

        let mut state = StudioState::seed_default(&catalog);
        let now = chrono::Utc::now();
        let old = (now - chrono::Duration::days(45)).to_rfc3339();
        let recent = (now - chrono::Duration::days(5)).to_rfc3339();

        let space = &mut state.spaces[0];
        space.tabs = vec![
            crate::TabState {
                id: "t-active".into(),
                title: "Active Old".into(),
                database: Some("db1".into()),
                content: "SELECT 1".into(),
                is_pinned: false,
                group_id: None,
                last_accessed_at: old.clone(),
            },
            crate::TabState {
                id: "t-pinned".into(),
                title: "Pinned Old".into(),
                database: Some("db1".into()),
                content: "SELECT 2".into(),
                is_pinned: true,
                group_id: None,
                last_accessed_at: old.clone(),
            },
            crate::TabState {
                id: "t-stale".into(),
                title: "Stale Tab".into(),
                database: Some("db1".into()),
                content: "SELECT 3".into(),
                is_pinned: false,
                group_id: None,
                last_accessed_at: old,
            },
            crate::TabState {
                id: "t-recent".into(),
                title: "Recent Tab".into(),
                database: Some("db1".into()),
                content: "SELECT 4".into(),
                is_pinned: false,
                group_id: None,
                last_accessed_at: recent,
            },
        ];
        space.active_tab_id = Some("t-active".into());
        state.auto_archive_enabled = true;
        state.auto_archive_days = 30;

        let archived = state.run_auto_archive(now);
        assert_eq!(archived, vec!["t-stale".to_string()]);
        assert_eq!(state.archived_tabs.len(), 1);
        assert_eq!(state.archived_tabs[0].id, "t-stale");
        assert_eq!(state.spaces[0].tabs.len(), 3);
    }

    #[test]
    fn notes_crud_archive_survival_and_orphan_pruning() {
        let dir = std::env::temp_dir().join(format!("larik_store_notes_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("test.db");

        let mut catalog = ProviderCatalog::new();
        catalog.register(Arc::new(TestSqlProvider));

        let mut state = StudioState::seed_default(&catalog);
        let space_id = state.spaces[0].id.clone();
        let live_tab_id = state.spaces[0].tabs[0].id.clone();
        let archived_tab_id = state.spaces[0].tabs[1].id.clone();

        let store =
            StudioStore::open_at_with_secrets(db, Arc::new(InMemorySecretBackend::new())).unwrap();
        store.save(&state, &catalog).unwrap();

        let mut n1 = NoteRecord::new(
            &space_id,
            &live_tab_id,
            crate::notes::capture_anchor("SELECT 1;\nSELECT 2;", 10, 9),
        );
        n1.kind = NoteKind::Decision;
        n1.body = "Keep this query read-only".into();

        let n2 = NoteRecord::new(&space_id, &archived_tab_id, crate::notes::tab_note_anchor());
        let orphan = NoteRecord::new(&space_id, "deleted-tab-id", crate::notes::tab_note_anchor());

        store.save_notes_batch(&[n1.clone(), n2.clone(), orphan]).unwrap();
        assert_eq!(store.get_notes_for_tab(&live_tab_id).unwrap().len(), 1);
        assert_eq!(store.get_notes_for_space(&space_id).unwrap().len(), 3);

        // Move second tab into archived_tabs and save snapshot:
        // live_tab + archived_tab notes MUST survive, while deleted-tab-id note MUST be pruned.
        let moved = state.spaces[0].tabs.remove(1);
        state.archived_tabs.push(crate::ArchivedTabState {
            id: moved.id,
            space_id: space_id.clone(),
            space_name: Some(state.spaces[0].name.clone()),
            color: Some(state.spaces[0].color.clone()),
            title: moved.title,
            database: moved.database,
            content: moved.content,
            is_pinned: false,
            group_name: None,
            archived_at: crate::now_rfc3339(),
        });
        store.save_snapshot_only(&state, &catalog).unwrap();

        assert_eq!(store.get_notes_for_tab(&live_tab_id).unwrap().len(), 1);
        assert_eq!(store.get_notes_for_tab(&archived_tab_id).unwrap().len(), 1);
        assert!(store.get_notes_for_tab("deleted-tab-id").unwrap().is_empty());

        // Reassign archived tab's notes to a new space on restore.
        store.reassign_notes_space(&archived_tab_id, "new-space").unwrap();
        let reassigned = store.get_notes_for_tab(&archived_tab_id).unwrap();
        assert_eq!(reassigned[0].space_id, "new-space");

        // Deleting a single note and deleting all notes for a tab.
        store.delete_note(&n1.id).unwrap();
        assert!(store.get_notes_for_tab(&live_tab_id).unwrap().is_empty());
        store.delete_notes_for_tab(&archived_tab_id).unwrap();
        assert!(store.get_notes_for_tab(&archived_tab_id).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_placement_round_trips_through_snapshot() {
        let dir = std::env::temp_dir().join(format!("larik_win_bounds_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("studio.db");

        let catalog = ProviderCatalog::new();
        let store =
            StudioStore::open_at_with_secrets(db, Arc::new(InMemorySecretBackend::new())).unwrap();
        assert!(store.load_window_placement().is_none());

        let mut state = StudioState::seed_default(&catalog);
        let placement = crate::WindowPlacementState {
            x: 180.0,
            y: 95.0,
            width: 1480.0,
            height: 920.0,
            mode: crate::WindowMode::Maximized,
            display_uuid: Some("display-2-uuid".into()),
        };
        state.window_placement = Some(placement.clone());
        store.save_snapshot_only(&state, &catalog).unwrap();

        assert_eq!(store.load_window_placement(), Some(placement.clone()));
        assert_eq!(store.load(&catalog).window_placement, Some(placement));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
