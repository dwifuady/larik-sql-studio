//! Automatic, idempotent backward-compatible migration from the legacy Tauri Larik SQL Studio
//! SQLite database (`larik.db` / `larik-dev.db`) and Windows Credential Manager (`keyring` v3)
//! into the GPUI `StudioStore` schema.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use chrono::{DateTime, NaiveDateTime, Utc};
use directories::ProjectDirs;
use larik_data::ConnectionProfile;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::json;

use crate::mcp::{MCP_MAX_STATEMENT_CHARS, McpAccess, mcp_approval_status, truncate_chars};
use crate::notes::{NoteKind, NoteRecord, capture_anchor, tab_note_anchor};
use crate::run_log::{
    normalize_retention_days, run_actor, run_status, truncate_statement,
};
use crate::secrets::{SecretBackend, legacy_tauri_secret_target, secret_target};
use crate::snippets::{
    SnippetRecord, builtin_catalog, builtin_id, normalize_dialects,
};
use crate::{
    ArchivedTabState, DEFAULT_AUTO_ARCHIVE_DAYS, MAX_AUTO_ARCHIVE_DAYS, MIN_AUTO_ARCHIVE_DAYS,
    SPACE_COLORS, SpaceState, StudioState, TabGroupState, TabState, ThemePreference, now_rfc3339,
};

pub const TAURI_MIGRATION_META_KEY: &str = "tauri_migration_v1";

/// Summary returned when a legacy Tauri database is migrated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TauriMigrationReport {
    pub source_path: PathBuf,
    pub backup_path: PathBuf,
    pub in_place: bool,
    pub spaces_migrated: usize,
    pub tabs_migrated: usize,
    pub folders_migrated: usize,
    pub archived_tabs_migrated: usize,
    pub snippets_migrated: usize,
    pub notes_migrated: usize,
    pub secrets_migrated: usize,
    pub mcp_log_migrated: usize,
    pub mcp_approvals_migrated: usize,
    pub run_log_migrated: usize,
}

/// Default legacy Tauri database locations (`%APPDATA%\larik\larik-sql-studio\data\larik.db`
/// on Windows, plus `larik-dev.db` as a fallback in debug builds).
pub fn default_legacy_tauri_db_candidates() -> Vec<PathBuf> {
    let Some(dirs) = ProjectDirs::from("com", "larik", "larik-sql-studio") else {
        return Vec::new();
    };
    let data_dir = dirs.data_dir();
    let mut candidates = vec![data_dir.join("larik.db")];
    #[cfg(debug_assertions)]
    candidates.push(data_dir.join("larik-dev.db"));
    candidates
}

/// Sibling legacy Tauri database filenames in the same directory as `db_path`.
pub fn sibling_legacy_tauri_db_candidates(db_path: &Path) -> Vec<PathBuf> {
    let Some(parent) = db_path.parent() else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for name in ["larik.db", "larik-dev.db"] {
        let candidate = parent.join(name);
        if candidate != db_path {
            candidates.push(candidate);
        }
    }
    candidates
}

#[derive(Debug, Default)]
struct LegacySpaceRow {
    id: String,
    name: String,
    color: Option<String>,
    icon: Option<String>,
    host: Option<String>,
    port: Option<i64>,
    database: Option<String>,
    username: Option<String>,
    password: Option<String>,
    trust_cert: bool,
    encrypt: bool,
    mcp_access: Option<String>,
    last_active_tab_id: Option<String>,
    driver: Option<String>,
}

#[derive(Debug, Default)]
struct LegacyFolderRow {
    id: String,
    space_id: String,
    name: String,
    is_expanded: bool,
}

#[derive(Debug, Default)]
struct LegacyTabRow {
    id: String,
    space_id: String,
    title: String,
    tab_type: Option<String>,
    content: Option<String>,
    database: Option<String>,
    folder_id: Option<String>,
    is_pinned: bool,
    last_accessed_at: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Default)]
struct LegacyArchivedTabRow {
    id: String,
    original_tab_id: Option<String>,
    space_id: String,
    space_name: Option<String>,
    title: String,
    tab_type: Option<String>,
    content: Option<String>,
    database: Option<String>,
    was_pinned: bool,
    archived_at: Option<String>,
}

#[derive(Debug, Default)]
struct LegacySnippetRow {
    id: String,
    trigger: String,
    name: String,
    content: String,
    description: Option<String>,
    is_builtin: bool,
    enabled: bool,
    category: Option<String>,
    drivers: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Default)]
struct LegacyStickyNoteRow {
    id: String,
    tab_id: String,
    line_number: i64,
    content: String,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Default)]
struct LegacyMcpLogRow {
    ts: Option<String>,
    tool: String,
    connection_id: Option<String>,
    space_name: Option<String>,
    statement: Option<String>,
    access_level: Option<String>,
    status: String,
    duration_ms: i64,
    rows_returned: i64,
}

#[derive(Debug, Default)]
struct LegacyMcpApprovalRow {
    id: String,
    ts: Option<String>,
    connection_id: Option<String>,
    space_name: Option<String>,
    statement: String,
    status: String,
    decided_at: Option<String>,
}

#[derive(Debug, Default)]
struct LegacyTauriData {
    spaces: Vec<LegacySpaceRow>,
    folders: Vec<LegacyFolderRow>,
    tabs: Vec<LegacyTabRow>,
    archived_tabs: Vec<LegacyArchivedTabRow>,
    snippets: Vec<LegacySnippetRow>,
    sticky_notes: Vec<LegacyStickyNoteRow>,
    app_state: HashMap<String, String>,
    mcp_log: Vec<LegacyMcpLogRow>,
    mcp_approvals: Vec<LegacyMcpApprovalRow>,
}

impl LegacyTauriData {
    fn has_any_migratable_data(&self) -> bool {
        !self.spaces.is_empty()
            || !self.archived_tabs.is_empty()
            || self.snippets.iter().any(|s| !s.is_builtin || !s.enabled)
            || !self.sticky_notes.is_empty()
            || !self.mcp_log.is_empty()
            || !self.mcp_approvals.is_empty()
            || !self.app_state.is_empty()
    }
}

/// Checks if `db_path` itself is an unmigrated legacy Tauri database.
/// If so, creates a backup, reads all legacy data into memory, renames conflicting legacy
/// tables (`mcp_log`, `mcp_approvals`), removes old UUID built-in snippets, and returns the
/// staged migration payload so the caller can run `ensure_schema()` and then commit it.
pub(crate) struct StagedInPlaceMigration {
    source_path: PathBuf,
    backup_path: PathBuf,
    data: LegacyTauriData,
}

pub(crate) fn prepare_in_place_migration(
    db_path: &Path,
) -> anyhow::Result<Option<StagedInPlaceMigration>> {
    if !db_path.is_file() {
        return Ok(None);
    }

    let conn = open_read_write(db_path)?;
    if !is_unmigrated_tauri_db(&conn)? {
        return Ok(None);
    }

    let data = read_legacy_data(&conn)?;
    if !data.has_any_migratable_data() {
        return Ok(None);
    }

    let backup_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let backup_path = create_safe_backup(db_path, backup_dir)?;

    reconcile_in_place_schema(&conn)?;

    Ok(Some(StagedInPlaceMigration {
        source_path: db_path.to_path_buf(),
        backup_path,
        data,
    }))
}

/// Completes a previously staged in-place migration after `ensure_schema()` has run.
pub(crate) fn finish_in_place_migration(
    db_path: &Path,
    secrets: &Arc<dyn SecretBackend>,
    staged: StagedInPlaceMigration,
) -> anyhow::Result<TauriMigrationReport> {
    let mut conn = open_read_write(db_path)?;
    write_migrated_data(
        &mut conn,
        secrets,
        &staged.source_path,
        &staged.backup_path,
        true,
        staged.data,
    )
}

/// Checks whether `db_path` is eligible for migration from an external legacy Tauri DB
/// (i.e. has no `tauri_migration_v1` marker and no `studio_snapshot` row yet) and migrates
/// the first matching legacy candidate found in `candidates`.
pub(crate) fn migrate_from_external_candidates(
    db_path: &Path,
    secrets: &Arc<dyn SecretBackend>,
    candidates: &[PathBuf],
) -> anyhow::Result<Option<TauriMigrationReport>> {
    if candidates.is_empty() {
        return Ok(None);
    }

    let mut target_conn = open_read_write(db_path)?;
    if target_already_initialized(&target_conn)? {
        return Ok(None);
    }

    let target_canonical = std::fs::canonicalize(db_path).ok();

    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        if candidate == db_path {
            continue;
        }
        if let (Some(tc), Ok(cc)) = (&target_canonical, std::fs::canonicalize(candidate)) {
            if tc == &cc {
                continue;
            }
        }

        let source_conn = match open_legacy_readonly_or_rw(candidate) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if !is_unmigrated_tauri_db(&source_conn).unwrap_or(false) {
            continue;
        }
        let data = match read_legacy_data(&source_conn) {
            Ok(d) if d.has_any_migratable_data() => d,
            _ => continue,
        };
        drop(source_conn);

        let backup_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
        let backup_path = create_safe_backup(candidate, backup_dir)?;

        let report = write_migrated_data(
            &mut target_conn,
            secrets,
            candidate,
            &backup_path,
            false,
            data,
        )?;
        return Ok(Some(report));
    }

    Ok(None)
}

fn open_read_write(path: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open SQLite database {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

fn open_legacy_readonly_or_rw(path: &Path) -> anyhow::Result<Connection> {
    match Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => {
            let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
            Ok(conn)
        }
        Err(_) => open_read_write(path),
    }
}

fn table_exists(conn: &Connection, table: &str) -> anyhow::Result<bool> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            params![table],
            |r| r.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn table_columns(conn: &Connection, table: &str) -> anyhow::Result<HashSet<String>> {
    let mut cols = HashSet::new();
    let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\""));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for col in rows {
        cols.insert(col?.to_ascii_lowercase());
    }
    Ok(cols)
}

fn target_already_initialized(conn: &Connection) -> anyhow::Result<bool> {
    if table_exists(conn, "studio_meta")? {
        let marker: Option<String> = conn
            .query_row(
                "SELECT value FROM studio_meta WHERE key = ?1",
                params![TAURI_MIGRATION_META_KEY],
                |r| r.get(0),
            )
            .optional()?;
        if marker.is_some() {
            return Ok(true);
        }
    }
    if table_exists(conn, "studio_snapshot")? {
        let snap: Option<i64> = conn
            .query_row("SELECT 1 FROM studio_snapshot WHERE id = 1", [], |r| r.get(0))
            .optional()?;
        if snap.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_unmigrated_tauri_db(conn: &Connection) -> anyhow::Result<bool> {
    if target_already_initialized(conn)? {
        return Ok(false);
    }
    let has_spaces = table_exists(conn, "spaces")?;
    if !has_spaces {
        return Ok(false);
    }
    let has_pinned_tabs = table_exists(conn, "pinned_tabs")?;
    let has_app_state = table_exists(conn, "app_state")?;
    Ok(has_pinned_tabs || has_app_state)
}

fn unique_backup_path(source_path: &Path, backup_dir: &Path) -> PathBuf {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("larik");
    let first = backup_dir.join(format!("{stem}.tauri-backup.db"));
    if !first.exists() {
        return first;
    }
    for idx in 1..=9999 {
        let candidate = backup_dir.join(format!("{stem}.tauri-backup-{idx}.db"));
        if !candidate.exists() {
            return candidate;
        }
    }
    backup_dir.join(format!(
        "{stem}.tauri-backup-{}.db",
        uuid::Uuid::new_v4().simple()
    ))
}

fn create_safe_backup(source_path: &Path, backup_dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(backup_dir)
        .with_context(|| format!("Failed to create backup directory {}", backup_dir.display()))?;
    let backup_path = unique_backup_path(source_path, backup_dir);

    // Prefer SQLite `VACUUM INTO` so any uncommitted WAL frames are safely captured into a
    // self-contained single-file backup without mutating the source DB.
    let vacuum_ok = open_legacy_readonly_or_rw(source_path)
        .and_then(|conn| {
            let escaped = backup_path.to_string_lossy().replace('\'', "''");
            conn.execute(&format!("VACUUM INTO '{escaped}'"), [])
                .map_err(anyhow::Error::from)
        })
        .is_ok();

    if !vacuum_ok || !backup_path.is_file() {
        let _ = std::fs::remove_file(&backup_path);
        std::fs::copy(source_path, &backup_path).with_context(|| {
            format!(
                "Failed to copy legacy database {} to backup {}",
                source_path.display(),
                backup_path.display()
            )
        })?;
    }

    Ok(backup_path)
}

fn reconcile_in_place_schema(conn: &Connection) -> anyhow::Result<()> {
    // Rename legacy `mcp_log` / `mcp_approvals` tables whose column definitions differ from
    // the GPUI schema so `ensure_schema()` can create the new tables cleanly.
    if table_exists(conn, "mcp_log")? {
        let cols = table_columns(conn, "mcp_log")?;
        if !cols.contains("recorded_at") {
            let renamed = find_unused_table_name(conn, "legacy_tauri_mcp_log")?;
            conn.execute(&format!("ALTER TABLE mcp_log RENAME TO \"{renamed}\""), [])?;
        }
    }

    if table_exists(conn, "mcp_approvals")? {
        let cols = table_columns(conn, "mcp_approvals")?;
        if !cols.contains("created_at") {
            let renamed = find_unused_table_name(conn, "legacy_tauri_mcp_approvals")?;
            conn.execute(
                &format!("ALTER TABLE mcp_approvals RENAME TO \"{renamed}\""),
                [],
            )?;
        }
    }

    if table_exists(conn, "snippets")? {
        // Legacy Tauri seeded built-ins with random UUIDs; drop those so we can re-insert
        // under deterministic `builtin:{scope}:{trigger}` IDs without duplicates.
        conn.execute(
            "DELETE FROM snippets WHERE is_builtin = 1 AND id NOT LIKE 'builtin:%'",
            [],
        )?;
    }

    Ok(())
}

fn find_unused_table_name(conn: &Connection, base: &str) -> anyhow::Result<String> {
    if !table_exists(conn, base)? {
        return Ok(base.to_string());
    }
    for idx in 1..=1000 {
        let candidate = format!("{base}_{idx}");
        if !table_exists(conn, &candidate)? {
            return Ok(candidate);
        }
    }
    Ok(format!("{base}_{}", uuid::Uuid::new_v4().simple()))
}

fn col_expr(cols: &HashSet<String>, name: &str, fallback_sql: &str) -> String {
    if cols.contains(&name.to_ascii_lowercase()) {
        name.to_string()
    } else {
        format!("{fallback_sql} AS {name}")
    }
}

fn read_legacy_data(conn: &Connection) -> anyhow::Result<LegacyTauriData> {
    let mut data = LegacyTauriData::default();

    if table_exists(conn, "spaces")? {
        let cols = table_columns(conn, "spaces")?;
        let order_clause = if cols.contains("sort_order") && cols.contains("created_at") {
            "ORDER BY sort_order ASC, created_at ASC, id ASC"
        } else if cols.contains("sort_order") {
            "ORDER BY sort_order ASC, id ASC"
        } else {
            "ORDER BY id ASC"
        };
        let sql = format!(
            "SELECT id, name, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM spaces {order_clause}",
            col_expr(&cols, "color", "NULL"),
            col_expr(&cols, "icon", "NULL"),
            col_expr(&cols, "connection_host", "NULL"),
            col_expr(&cols, "connection_port", "1433"),
            col_expr(&cols, "connection_database", "NULL"),
            col_expr(&cols, "connection_username", "NULL"),
            col_expr(&cols, "connection_password", "NULL"),
            col_expr(&cols, "connection_trust_cert", "1"),
            col_expr(&cols, "connection_encrypt", "0"),
            col_expr(&cols, "mcp_access", "'off'"),
            col_expr(&cols, "last_active_tab_id", "NULL"),
            col_expr(&cols, "connection_driver", "'mssql'"),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let trust_cert: Option<i64> = row.get(9)?;
            let encrypt: Option<i64> = row.get(10)?;
            Ok(LegacySpaceRow {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                icon: row.get(3)?,
                host: row.get(4)?,
                port: row.get(5)?,
                database: row.get(6)?,
                username: row.get(7)?,
                password: row.get(8)?,
                trust_cert: trust_cert.unwrap_or(1) != 0,
                encrypt: encrypt.unwrap_or(0) != 0,
                mcp_access: row.get(11)?,
                last_active_tab_id: row.get(12)?,
                driver: row.get(13)?,
            })
        })?;
        for row in rows {
            data.spaces.push(row?);
        }
    }

    if table_exists(conn, "tab_folders")? {
        let cols = table_columns(conn, "tab_folders")?;
        let order_clause = if cols.contains("sort_order") && cols.contains("created_at") {
            "ORDER BY sort_order ASC, created_at ASC, id ASC"
        } else if cols.contains("sort_order") {
            "ORDER BY sort_order ASC, id ASC"
        } else {
            "ORDER BY id ASC"
        };
        let sql = format!(
            "SELECT id, space_id, name, {} FROM tab_folders {order_clause}",
            col_expr(&cols, "is_expanded", "1"),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let expanded: Option<i64> = row.get(3)?;
            Ok(LegacyFolderRow {
                id: row.get(0)?,
                space_id: row.get(1)?,
                name: row.get(2)?,
                is_expanded: expanded.unwrap_or(1) != 0,
            })
        })?;
        for row in rows {
            data.folders.push(row?);
        }
    }

    if table_exists(conn, "pinned_tabs")? {
        let cols = table_columns(conn, "pinned_tabs")?;
        let order_clause = if cols.contains("sort_order") && cols.contains("created_at") {
            "ORDER BY sort_order ASC, created_at ASC, id ASC"
        } else if cols.contains("sort_order") {
            "ORDER BY sort_order ASC, id ASC"
        } else {
            "ORDER BY id ASC"
        };
        let sql = format!(
            "SELECT id, space_id, title, {}, {}, {}, {}, {}, {}, {}, {} FROM pinned_tabs {order_clause}",
            col_expr(&cols, "tab_type", "'query'"),
            col_expr(&cols, "content", "''"),
            col_expr(&cols, "database", "NULL"),
            col_expr(&cols, "folder_id", "NULL"),
            col_expr(&cols, "is_pinned", "0"),
            col_expr(&cols, "last_accessed_at", "NULL"),
            col_expr(&cols, "created_at", "NULL"),
            col_expr(&cols, "updated_at", "NULL"),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let pinned: Option<i64> = row.get(7)?;
            Ok(LegacyTabRow {
                id: row.get(0)?,
                space_id: row.get(1)?,
                title: row.get(2)?,
                tab_type: row.get(3)?,
                content: row.get(4)?,
                database: row.get(5)?,
                folder_id: row.get(6)?,
                is_pinned: pinned.unwrap_or(0) != 0,
                last_accessed_at: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?;
        for row in rows {
            data.tabs.push(row?);
        }
    }

    if table_exists(conn, "archived_tabs")? {
        let cols = table_columns(conn, "archived_tabs")?;
        let order_clause = if cols.contains("archived_at") {
            "ORDER BY archived_at DESC, id DESC"
        } else {
            "ORDER BY id DESC"
        };
        let sql = format!(
            "SELECT id, {}, space_id, {}, title, {}, {}, {}, {}, {} FROM archived_tabs {order_clause}",
            col_expr(&cols, "original_tab_id", "NULL"),
            col_expr(&cols, "space_name", "NULL"),
            col_expr(&cols, "tab_type", "'query'"),
            col_expr(&cols, "content", "''"),
            col_expr(&cols, "database", "NULL"),
            col_expr(&cols, "was_pinned", "0"),
            col_expr(&cols, "archived_at", "NULL"),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let was_pinned: Option<i64> = row.get(8)?;
            Ok(LegacyArchivedTabRow {
                id: row.get(0)?,
                original_tab_id: row.get(1)?,
                space_id: row.get(2)?,
                space_name: row.get(3)?,
                title: row.get(4)?,
                tab_type: row.get(5)?,
                content: row.get(6)?,
                database: row.get(7)?,
                was_pinned: was_pinned.unwrap_or(0) != 0,
                archived_at: row.get(9)?,
            })
        })?;
        for row in rows {
            data.archived_tabs.push(row?);
        }
    }

    if table_exists(conn, "snippets")? {
        let cols = table_columns(conn, "snippets")?;
        let sql = format!(
            "SELECT id, trigger, name, content, {}, {}, {}, {}, {}, {}, {} FROM snippets",
            col_expr(&cols, "description", "NULL"),
            col_expr(&cols, "is_builtin", "0"),
            col_expr(&cols, "enabled", "1"),
            col_expr(&cols, "category", "NULL"),
            col_expr(&cols, "drivers", "NULL"),
            col_expr(&cols, "created_at", "NULL"),
            col_expr(&cols, "updated_at", "NULL"),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let is_builtin: Option<i64> = row.get(5)?;
            let enabled: Option<i64> = row.get(6)?;
            Ok(LegacySnippetRow {
                id: row.get(0)?,
                trigger: row.get(1)?,
                name: row.get(2)?,
                content: row.get(3)?,
                description: row.get(4)?,
                is_builtin: is_builtin.unwrap_or(0) != 0,
                enabled: enabled.unwrap_or(1) != 0,
                category: row.get(7)?,
                drivers: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?;
        for row in rows {
            data.snippets.push(row?);
        }
    }

    if table_exists(conn, "sticky_notes")? {
        let cols = table_columns(conn, "sticky_notes")?;
        let order_clause = if cols.contains("created_at") {
            "ORDER BY created_at ASC, id ASC"
        } else {
            "ORDER BY id ASC"
        };
        let sql = format!(
            "SELECT id, tab_id, {}, content, {}, {} FROM sticky_notes {order_clause}",
            col_expr(&cols, "line_number", "0"),
            col_expr(&cols, "created_at", "NULL"),
            col_expr(&cols, "updated_at", "NULL"),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let line_number: Option<i64> = row.get(2)?;
            Ok(LegacyStickyNoteRow {
                id: row.get(0)?,
                tab_id: row.get(1)?,
                line_number: line_number.unwrap_or(0),
                content: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        for row in rows {
            data.sticky_notes.push(row?);
        }
    }

    if table_exists(conn, "app_state")? {
        let mut stmt = conn.prepare("SELECT key, value FROM app_state")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (k, v) = row?;
            data.app_state.insert(k, v);
        }
    }

    if table_exists(conn, "mcp_log")? {
        let cols = table_columns(conn, "mcp_log")?;
        if cols.contains("ts") {
            let sql = format!(
                "SELECT ts, tool, {}, {}, {}, {}, status, {}, {} FROM mcp_log ORDER BY ts ASC, rowid ASC",
                col_expr(&cols, "connection_id", "NULL"),
                col_expr(&cols, "space_name", "NULL"),
                col_expr(&cols, "statement", "NULL"),
                col_expr(&cols, "access_level", "'off'"),
                col_expr(&cols, "duration_ms", "0"),
                col_expr(&cols, "rows_returned", "0"),
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                let duration_ms: Option<i64> = row.get(7)?;
                let rows_returned: Option<i64> = row.get(8)?;
                Ok(LegacyMcpLogRow {
                    ts: row.get(0)?,
                    tool: row.get(1)?,
                    connection_id: row.get(2)?,
                    space_name: row.get(3)?,
                    statement: row.get(4)?,
                    access_level: row.get(5)?,
                    status: row.get(6)?,
                    duration_ms: duration_ms.unwrap_or(0),
                    rows_returned: rows_returned.unwrap_or(0),
                })
            })?;
            for row in rows {
                data.mcp_log.push(row?);
            }
        }
    }

    if table_exists(conn, "mcp_approvals")? {
        let cols = table_columns(conn, "mcp_approvals")?;
        if cols.contains("ts") {
            let sql = format!(
                "SELECT id, ts, {}, {}, statement, status, {} FROM mcp_approvals ORDER BY ts ASC, rowid ASC",
                col_expr(&cols, "connection_id", "''"),
                col_expr(&cols, "space_name", "NULL"),
                col_expr(&cols, "decided_at", "NULL"),
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                Ok(LegacyMcpApprovalRow {
                    id: row.get(0)?,
                    ts: row.get(1)?,
                    connection_id: row.get(2)?,
                    space_name: row.get(3)?,
                    statement: row.get(4)?,
                    status: row.get(5)?,
                    decided_at: row.get(6)?,
                })
            })?;
            for row in rows {
                data.mcp_approvals.push(row?);
            }
        }
    }

    Ok(data)
}

fn write_migrated_data(
    conn: &mut Connection,
    secrets: &Arc<dyn SecretBackend>,
    source_path: &Path,
    backup_path: &Path,
    in_place: bool,
    data: LegacyTauriData,
) -> anyhow::Result<TauriMigrationReport> {
    let mut report = TauriMigrationReport {
        source_path: source_path.to_path_buf(),
        backup_path: backup_path.to_path_buf(),
        in_place,
        spaces_migrated: 0,
        tabs_migrated: 0,
        folders_migrated: 0,
        archived_tabs_migrated: 0,
        snippets_migrated: 0,
        notes_migrated: 0,
        secrets_migrated: 0,
        mcp_log_migrated: 0,
        mcp_approvals_migrated: 0,
        run_log_migrated: 0,
    };

    // Index folders and tabs by space_id.
    let mut folders_by_space: HashMap<String, Vec<LegacyFolderRow>> = HashMap::new();
    for folder in data.folders {
        folders_by_space
            .entry(folder.space_id.clone())
            .or_default()
            .push(folder);
    }

    let mut tabs_by_space: HashMap<String, Vec<LegacyTabRow>> = HashMap::new();
    for tab in data.tabs {
        if !is_query_tab_type(tab.tab_type.as_deref()) {
            continue;
        }
        tabs_by_space
            .entry(tab.space_id.clone())
            .or_default()
            .push(tab);
    }

    // Track tab metadata for note anchoring & space lookup.
    struct TabNoteTarget {
        migrated_tab_id: String,
        space_id: String,
        content: String,
    }
    let mut tab_note_targets: HashMap<String, TabNoteTarget> = HashMap::new();
    let mut space_info_map: HashMap<String, (String, String, String)> = HashMap::new(); // id -> (name, color, provider_id)
    let mut space_mcp_access_rows: Vec<(String, McpAccess)> = Vec::new();
    let mut secrets_to_migrate: Vec<(String, String)> = Vec::new();

    let mut spaces: Vec<SpaceState> = Vec::new();
    let mut active_tab_ids: HashSet<String> = HashSet::new();
    let mut max_query_num: u32 = 0;

    for (idx, legacy_space) in data.spaces.into_iter().enumerate() {
        let provider_id = normalize_provider_id(legacy_space.driver.as_deref());
        let color = non_empty_trimmed(legacy_space.color.as_deref()).unwrap_or_else(|| {
            SPACE_COLORS[idx % SPACE_COLORS.len()].1.to_string()
        });
        let icon = non_empty_trimmed(legacy_space.icon.as_deref());
        let active_database = non_empty_trimmed(legacy_space.database.as_deref());

        let mut profile = ConnectionProfile::new(&provider_id);
        let host = non_empty_trimmed(legacy_space.host.as_deref());
        let user = non_empty_trimmed(legacy_space.username.as_deref());
        let has_conn_config = host.is_some() || active_database.is_some() || user.is_some();
        if has_conn_config {
            if let Some(h) = host {
                profile.set("server", h);
            }
            if let Some(port) = legacy_space.port.filter(|p| *p > 0) {
                profile.set("port", port.to_string());
            } else if provider_id == "mssql" {
                profile.set("port", "1433");
            }
            if let Some(ref db) = active_database {
                profile.set("database", db.clone());
            }
            if let Some(u) = user {
                profile.set("user", u);
            }
            profile.set(
                "encrypt",
                if legacy_space.encrypt { "true" } else { "false" },
            );
            profile.set(
                "trustServerCertificate",
                if legacy_space.trust_cert {
                    "true"
                } else {
                    "false"
                },
            );
        }

        // Resolve password from legacy Tauri keyring first, falling back to SQLite column.
        let legacy_keyring_pw = secrets
            .get_legacy_keyring(&legacy_tauri_secret_target(&legacy_space.id))
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());
        let legacy_sqlite_pw = legacy_space.password.filter(|s| !s.is_empty());
        if let Some(pw) = legacy_keyring_pw.or(legacy_sqlite_pw) {
            secrets_to_migrate.push((legacy_space.id.clone(), pw));
        }

        let mcp_access = McpAccess::parse_fail_closed(legacy_space.mcp_access.as_deref());
        space_mcp_access_rows.push((legacy_space.id.clone(), mcp_access));

        let legacy_folders = folders_by_space
            .remove(&legacy_space.id)
            .unwrap_or_default();
        let valid_folder_ids: HashSet<String> =
            legacy_folders.iter().map(|f| f.id.clone()).collect();

        let legacy_tabs = tabs_by_space.remove(&legacy_space.id).unwrap_or_default();
        let mut pinned_folder_ids: HashSet<String> = HashSet::new();
        let mut migrated_tabs: Vec<TabState> = Vec::new();

        for tab in legacy_tabs {
            let group_id = non_empty_trimmed(tab.folder_id.as_deref())
                .filter(|fid| valid_folder_ids.contains(fid));
            if let Some(ref gid) = group_id {
                if tab.is_pinned {
                    pinned_folder_ids.insert(gid.clone());
                }
            }
            let title = non_empty_trimmed(Some(&tab.title)).unwrap_or_else(|| "Query".to_string());
            if let Some(n) = parse_query_number(&title) {
                max_query_num = max_query_num.max(n);
            }
            let content = tab.content.unwrap_or_default();
            let last_accessed_at = to_rfc3339_utc(
                tab.last_accessed_at
                    .as_deref()
                    .or(tab.updated_at.as_deref())
                    .or(tab.created_at.as_deref()),
            );

            active_tab_ids.insert(tab.id.clone());
            tab_note_targets.insert(
                tab.id.clone(),
                TabNoteTarget {
                    migrated_tab_id: tab.id.clone(),
                    space_id: legacy_space.id.clone(),
                    content: content.clone(),
                },
            );

            migrated_tabs.push(TabState {
                id: tab.id,
                title,
                database: non_empty_trimmed(tab.database.as_deref()),
                content,
                is_pinned: tab.is_pinned,
                group_id,
                last_accessed_at,
            });
        }

        let migrated_groups: Vec<TabGroupState> = legacy_folders
            .into_iter()
            .map(|f| {
                let is_pinned = pinned_folder_ids.contains(&f.id);
                TabGroupState {
                    id: f.id,
                    name: non_empty_trimmed(Some(&f.name)).unwrap_or_else(|| "Group".to_string()),
                    color: None,
                    is_pinned,
                    is_expanded: f.is_expanded,
                    database: None,
                }
            })
            .collect();

        if migrated_tabs.is_empty() {
            let fallback_id = uuid::Uuid::new_v4().to_string();
            migrated_tabs.push(TabState {
                id: fallback_id,
                title: "Query 1".to_string(),
                database: active_database.clone(),
                content: String::new(),
                is_pinned: false,
                group_id: None,
                last_accessed_at: now_rfc3339(),
            });
        }

        let mut space = SpaceState {
            id: legacy_space.id.clone(),
            name: non_empty_trimmed(Some(&legacy_space.name))
                .unwrap_or_else(|| format!("Workspace {}", idx + 1)),
            color: color.clone(),
            icon,
            profile,
            active_database,
            active_tab_id: non_empty_trimmed(legacy_space.last_active_tab_id.as_deref()),
            tabs: migrated_tabs,
            groups: migrated_groups,
            is_hidden: false,
        };
        space.rebuild_tab_order();

        let active_valid = space
            .active_tab_id
            .as_deref()
            .is_some_and(|tid| space.tabs.iter().any(|t| t.id == tid));
        if !active_valid {
            space.active_tab_id = space.tabs.first().map(|t| t.id.clone());
        }

        report.tabs_migrated += space.tabs.len();
        report.folders_migrated += space.groups.len();
        space_info_map.insert(
            space.id.clone(),
            (space.name.clone(), space.color.clone(), provider_id),
        );
        spaces.push(space);
    }
    report.spaces_migrated = spaces.len();

    // Migrate archived tabs.
    let mut used_archived_ids: HashSet<String> = active_tab_ids.clone();
    let mut archived_tabs: Vec<ArchivedTabState> = Vec::new();
    for arch in data.archived_tabs {
        if !is_query_tab_type(arch.tab_type.as_deref()) {
            continue;
        }
        let orig_id = non_empty_trimmed(arch.original_tab_id.as_deref());
        let chosen_id = match orig_id.as_ref() {
            Some(oid) if !used_archived_ids.contains(oid) => oid.clone(),
            _ => arch.id.clone(),
        };
        used_archived_ids.insert(chosen_id.clone());

        let content = arch.content.unwrap_or_default();
        let space_meta = space_info_map.get(&arch.space_id);
        let space_name = non_empty_trimmed(arch.space_name.as_deref())
            .or_else(|| space_meta.map(|(name, _, _)| name.clone()));
        let color = space_meta.map(|(_, color, _)| color.clone());

        // Map both `original_tab_id` and `archived_tabs.id` so sticky notes still link cleanly.
        if let Some(oid) = orig_id {
            tab_note_targets.entry(oid).or_insert_with(|| TabNoteTarget {
                migrated_tab_id: chosen_id.clone(),
                space_id: arch.space_id.clone(),
                content: content.clone(),
            });
        }
        tab_note_targets
            .entry(arch.id.clone())
            .or_insert_with(|| TabNoteTarget {
                migrated_tab_id: chosen_id.clone(),
                space_id: arch.space_id.clone(),
                content: content.clone(),
            });

        archived_tabs.push(ArchivedTabState {
            id: chosen_id,
            space_id: arch.space_id,
            space_name,
            color,
            title: non_empty_trimmed(Some(&arch.title)).unwrap_or_else(|| "Query".to_string()),
            database: non_empty_trimmed(arch.database.as_deref()),
            content,
            is_pinned: arch.was_pinned,
            group_name: None,
            archived_at: to_rfc3339_utc(arch.archived_at.as_deref()),
        });
    }
    report.archived_tabs_migrated = archived_tabs.len();

    // Apply `app_state` preferences.
    let active_space_id = data
        .app_state
        .get("last_space_id")
        .and_then(|s| non_empty_trimmed(Some(s)))
        .filter(|sid| spaces.iter().any(|s| s.id == *sid))
        .or_else(|| spaces.first().map(|s| s.id.clone()));

    if let (Some(active_sid), Some(last_tid)) = (
        active_space_id.as_deref(),
        data.app_state
            .get("last_tab_id")
            .and_then(|s| non_empty_trimmed(Some(s))),
    ) {
        if let Some(space) = spaces.iter_mut().find(|s| s.id == active_sid) {
            if space.tabs.iter().any(|t| t.id == last_tid) {
                space.active_tab_id = Some(last_tid);
            }
        }
    }

    let auto_archive_enabled = data
        .app_state
        .get("auto_archive_enabled")
        .map(|v| parse_bool(v, true))
        .unwrap_or(true);
    let auto_archive_days = data
        .app_state
        .get("auto_archive_days")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|d| d.clamp(MIN_AUTO_ARCHIVE_DAYS, MAX_AUTO_ARCHIVE_DAYS))
        .unwrap_or(DEFAULT_AUTO_ARCHIVE_DAYS);
    let theme = data
        .app_state
        .get("theme")
        .map(|v| match v.trim().to_ascii_lowercase().as_str() {
            "light" => ThemePreference::Light,
            "system" => ThemePreference::System,
            _ => ThemePreference::Dark,
        })
        .unwrap_or(ThemePreference::Dark);

    let retention_days = data
        .app_state
        .get("history_retention_days")
        .and_then(|v| v.trim().parse::<i32>().ok())
        .map(normalize_retention_days);
    let mcp_enabled = data
        .app_state
        .get("mcp_enabled")
        .map(|v| parse_bool(v, false));

    let query_counter = max_query_num
        .max(report.tabs_migrated as u32)
        .max(1);

    let snapshot_json = if !spaces.is_empty() {
        let state = StudioState {
            spaces,
            active_space_id,
            archived_tabs,
            query_counter,
            theme,
            schema_pane_open: false,
            notes_pane_open: false,
            rail_open: true,
            auto_archive_enabled,
            auto_archive_days,
            window_placement: None,
        };
        Some(serde_json::to_string(&state)?)
    } else {
        None
    };

    // Map legacy sticky notes -> GPUI NoteRecord.
    let mut migrated_notes: Vec<NoteRecord> = Vec::new();
    for note in data.sticky_notes {
        let Some(target) = tab_note_targets.get(&note.tab_id) else {
            continue;
        };
        let anchor = if note.line_number >= 1 {
            line_byte_span(&target.content, note.line_number as usize)
                .filter(|&(_, len)| len > 0)
                .map(|(start, len)| capture_anchor(&target.content, start, len))
                .unwrap_or_else(tab_note_anchor)
        } else {
            tab_note_anchor()
        };
        let created_at = to_rfc3339_utc(note.created_at.as_deref());
        let updated_at = to_rfc3339_utc(
            note.updated_at
                .as_deref()
                .or(note.created_at.as_deref()),
        );
        migrated_notes.push(NoteRecord {
            id: note.id,
            space_id: target.space_id.clone(),
            tab_id: target.migrated_tab_id.clone(),
            kind: NoteKind::Note,
            body: note.content,
            anchor,
            is_unanchored: false,
            created_at,
            updated_at,
        });
    }
    report.notes_migrated = migrated_notes.len();

    // Map snippets (preserve user snippets; map legacy built-ins onto deterministic `builtin:*` IDs).
    let builtins = builtin_catalog();
    let mut migrated_snippets: Vec<(SnippetRecord, String, String)> = Vec::new();
    for snip in data.snippets {
        let dialects = parse_drivers_column(snip.drivers.as_deref());
        let created_at = to_sqlite_utc(snip.created_at.as_deref());
        let updated_at = to_sqlite_utc(
            snip.updated_at
                .as_deref()
                .or(snip.created_at.as_deref()),
        );

        if snip.is_builtin {
            let matched = builtins
                .iter()
                .find(|def| {
                    def.trigger.eq_ignore_ascii_case(&snip.trigger)
                        && normalize_dialects(def.dialects.clone()) == dialects
                })
                .or_else(|| {
                    let matches: Vec<_> = builtins
                        .iter()
                        .filter(|def| def.trigger.eq_ignore_ascii_case(&snip.trigger))
                        .collect();
                    if matches.len() == 1 {
                        Some(matches[0])
                    } else {
                        None
                    }
                });

            if let Some(def) = matched {
                migrated_snippets.push((
                    SnippetRecord {
                        id: builtin_id(def),
                        trigger: snip.trigger.trim().to_string(),
                        name: snip.name.trim().to_string(),
                        content: snip.content,
                        description: non_empty_trimmed(snip.description.as_deref()),
                        category: non_empty_trimmed(snip.category.as_deref()),
                        dialects: normalize_dialects(def.dialects.clone()),
                        is_builtin: true,
                        is_enabled: snip.enabled,
                    },
                    created_at,
                    updated_at,
                ));
            }
        } else {
            migrated_snippets.push((
                SnippetRecord {
                    id: snip.id,
                    trigger: snip.trigger.trim().to_string(),
                    name: snip.name.trim().to_string(),
                    content: snip.content,
                    description: non_empty_trimmed(snip.description.as_deref()),
                    category: non_empty_trimmed(snip.category.as_deref()),
                    dialects,
                    is_builtin: false,
                    is_enabled: snip.enabled,
                },
                created_at,
                updated_at,
            ));
        }
    }
    report.snippets_migrated = migrated_snippets.len();

    // Commit everything atomically into the target SQLite database.
    let now_rfc = now_rfc3339();
    let tx = conn.transaction()?;

    if let Some(json) = snapshot_json {
        tx.execute(
            "INSERT INTO studio_snapshot (id, version, payload, updated_at)
             VALUES (1, 1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
            params![json, now_rfc],
        )?;
    }

    for (space_id, access) in &space_mcp_access_rows {
        tx.execute(
            "INSERT INTO space_mcp_access (space_id, access) VALUES (?1, ?2)
             ON CONFLICT(space_id) DO UPDATE SET access = excluded.access",
            params![space_id, access.as_storage()],
        )?;
    }

    if let Some(days) = retention_days {
        tx.execute(
            "INSERT INTO studio_meta (key, value) VALUES ('run_log_retention_days', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![days.to_string()],
        )?;
    }

    if let Some(enabled) = mcp_enabled {
        tx.execute(
            "INSERT INTO studio_meta (key, value) VALUES ('mcp_enabled', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![if enabled { "true" } else { "false" }],
        )?;
    }

    for (rec, created_at, updated_at) in &migrated_snippets {
        let drivers = rec
            .dialects
            .as_deref()
            .filter(|d| !d.is_empty())
            .map(|d| d.join(","));
        tx.execute(
            "INSERT INTO snippets (
                id, trigger, name, content, description, is_builtin, enabled,
                category, drivers, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                trigger = excluded.trigger,
                name = excluded.name,
                content = excluded.content,
                description = excluded.description,
                is_builtin = excluded.is_builtin,
                enabled = excluded.enabled,
                category = excluded.category,
                drivers = excluded.drivers,
                updated_at = excluded.updated_at",
            params![
                rec.id,
                rec.trigger,
                rec.name,
                rec.content,
                rec.description,
                if rec.is_builtin { 1i64 } else { 0i64 },
                if rec.is_enabled { 1i64 } else { 0i64 },
                rec.category,
                drivers,
                created_at,
                updated_at,
            ],
        )?;
    }

    for note in &migrated_notes {
        tx.execute(
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
    }

    for entry in &data.mcp_log {
        let recorded_at = to_sqlite_utc(entry.ts.as_deref());
        let space_meta = entry
            .connection_id
            .as_deref()
            .and_then(|sid| space_info_map.get(sid));
        let space_name = non_empty_trimmed(entry.space_name.as_deref())
            .or_else(|| space_meta.map(|(n, _, _)| n.clone()));
        let provider_id = space_meta.map(|(_, _, p)| p.clone());
        let access_level = McpAccess::parse_fail_closed(entry.access_level.as_deref())
            .as_storage()
            .to_string();
        let stmt_trimmed = entry
            .statement
            .as_deref()
            .map(|s| truncate_chars(s, MCP_MAX_STATEMENT_CHARS));

        tx.execute(
            "INSERT INTO mcp_log (
                recorded_at, tool, space_id, space_name, provider_id, access_level,
                statement, status, reason, duration_ms, rows_returned, decision_surface
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, NULL)",
            params![
                recorded_at,
                entry.tool,
                entry.connection_id,
                space_name,
                provider_id,
                access_level,
                stmt_trimmed,
                entry.status,
                entry.duration_ms.max(0),
                entry.rows_returned.max(0),
            ],
        )?;
        report.mcp_log_migrated += 1;

        if let Some(ref statement) = entry.statement {
            if !statement.trim().is_empty() {
                let mapped_status = map_legacy_mcp_status_to_run_status(&entry.status);
                tx.execute(
                    "INSERT INTO run_log (
                        recorded_at, actor, space_id, space_name, database_name, provider_id,
                        tab_id, statement, status, reason, duration_ms, rows_returned,
                        row_limit, is_truncated, description, reference
                     ) VALUES (
                        ?1, ?2, ?3, ?4, NULL, ?5,
                        NULL, ?6, ?7, NULL, ?8, ?9,
                        NULL, 0, NULL, NULL
                     )",
                    params![
                        recorded_at,
                        run_actor::AGENT,
                        entry.connection_id,
                        space_name,
                        provider_id,
                        truncate_statement(statement),
                        mapped_status,
                        entry.duration_ms.max(0),
                        entry.rows_returned.max(0),
                    ],
                )?;
                report.run_log_migrated += 1;
            }
        }
    }

    for appr in &data.mcp_approvals {
        let created_at = to_sqlite_utc(appr.ts.as_deref());
        let is_pending = appr.status.eq_ignore_ascii_case(mcp_approval_status::PENDING);
        let status = if is_pending {
            mcp_approval_status::ORPHANED.to_string()
        } else {
            appr.status.clone()
        };
        let decided_at = if let Some(ref d) = appr.decided_at {
            Some(to_sqlite_utc(Some(d)))
        } else if is_pending {
            Some(to_sqlite_utc(None))
        } else {
            None
        };
        let space_id = appr.connection_id.clone().unwrap_or_default();
        let stmt_trimmed = truncate_chars(&appr.statement, MCP_MAX_STATEMENT_CHARS);

        tx.execute(
            "INSERT OR IGNORE INTO mcp_approvals (
                id, created_at, space_id, space_name, statement, status, decided_at, surface
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                appr.id,
                created_at,
                space_id,
                appr.space_name,
                stmt_trimmed,
                status,
                decided_at,
            ],
        )?;
        report.mcp_approvals_migrated += 1;
    }

    // Migrate passwords into the new OS secret target namespace without deleting old Tauri
    // credentials (preserving rollback safety alongside the DB backup).
    for (space_id, pw) in &secrets_to_migrate {
        let target = secret_target(space_id, "password");
        if secrets.set(&target, pw).is_ok() {
            report.secrets_migrated += 1;
        }
    }

    let marker_payload = json!({
        "migrated_at": now_rfc,
        "source_path": source_path.to_string_lossy(),
        "backup_path": backup_path.to_string_lossy(),
        "in_place": in_place,
        "spaces": report.spaces_migrated,
        "tabs": report.tabs_migrated,
        "folders": report.folders_migrated,
        "archived_tabs": report.archived_tabs_migrated,
        "snippets": report.snippets_migrated,
        "notes": report.notes_migrated,
        "secrets": report.secrets_migrated,
        "mcp_log": report.mcp_log_migrated,
        "mcp_approvals": report.mcp_approvals_migrated,
        "run_log": report.run_log_migrated,
    })
    .to_string();

    tx.execute(
        "INSERT INTO studio_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![TAURI_MIGRATION_META_KEY, marker_payload],
    )?;

    tx.commit()?;
    Ok(report)
}

fn non_empty_trimmed(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn is_query_tab_type(tab_type: Option<&str>) -> bool {
    match tab_type.map(str::trim).filter(|s| !s.is_empty()) {
        None => true,
        Some(t) => t.eq_ignore_ascii_case("query"),
    }
}

fn normalize_provider_id(driver: Option<&str>) -> String {
    match driver
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("mssql" | "sqlserver" | "sql_server") => "mssql".to_string(),
        Some("postgres" | "postgresql" | "pg") => "postgres".to_string(),
        Some("mongo" | "mongodb") => "mongodb".to_string(),
        Some(other) => other.to_string(),
    }
}

fn parse_bool(raw: &str, default: bool) -> bool {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => true,
        "false" | "0" | "no" | "off" => false,
        _ => default,
    }
}

fn parse_query_number(title: &str) -> Option<u32> {
    let rest = title.trim().strip_prefix("Query ")?;
    rest.trim().parse::<u32>().ok()
}

fn parse_drivers_column(value: Option<&str>) -> Option<Vec<String>> {
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

fn parse_timestamp_utc(raw: Option<&str>) -> Option<DateTime<Utc>> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
        }
    }
    None
}

fn to_rfc3339_utc(raw: Option<&str>) -> String {
    parse_timestamp_utc(raw)
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}

fn to_sqlite_utc(raw: Option<&str>) -> String {
    parse_timestamp_utc(raw)
        .unwrap_or_else(Utc::now)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Returns `(byte_start, byte_len)` for 1-based `line_number` in `text`, excluding trailing `\r`/`\n`.
fn line_byte_span(text: &str, line_number: usize) -> Option<(usize, usize)> {
    if line_number == 0 {
        return None;
    }
    let bytes = text.as_bytes();
    let mut current_line = 1usize;
    let mut line_start = 0usize;
    let mut i = 0usize;

    while i <= bytes.len() {
        let is_end = i == bytes.len();
        let is_lf = !is_end && bytes[i] == b'\n';
        let is_cr = !is_end && bytes[i] == b'\r';

        if is_end || is_lf || is_cr {
            if current_line == line_number {
                return Some((line_start, i.saturating_sub(line_start)));
            }
            if is_end {
                break;
            }
            if is_cr && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 1;
            }
            current_line += 1;
            line_start = i + 1;
        }
        i += 1;
    }
    None
}

fn map_legacy_mcp_status_to_run_status(status: &str) -> &'static str {
    match status.trim().to_ascii_lowercase().as_str() {
        "ok" | "success" | "approved" | "allowed" => run_status::OK,
        "denied" | "refused" | "blocked" | "forbidden" => run_status::REFUSED,
        "cancelled" | "canceled" | "timeout" | "timed_out" => run_status::CANCELLED,
        _ => run_status::FAILED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use larik_data::{
        ConnectionProfile, DatabaseConnection, DatabaseProvider, EditorLanguage, ProviderCatalog,
        ProviderSetting, ProviderSettingKind, sql_language::SqlLanguage,
    };

    use crate::secrets::{InMemorySecretBackend, decode_legacy_keyring_blob};
    use crate::store::StudioStore;

    struct DummyMssqlProvider;

    impl DatabaseProvider for DummyMssqlProvider {
        fn id(&self) -> &'static str {
            "mssql"
        }
        fn display_name(&self) -> &'static str {
            "SQL Server"
        }
        fn dialect(&self) -> &'static str {
            "mssql"
        }
        fn settings(&self) -> &'static [ProviderSetting] {
            static SETTINGS: [ProviderSetting; 2] = [
                ProviderSetting {
                    key: "server",
                    label: "Server",
                    placeholder: Some("localhost"),
                    kind: ProviderSettingKind::Text,
                    required: true,
                    default_value: Some("localhost"),
                    hint: None,
                    is_advanced: false,
                },
                ProviderSetting {
                    key: "password",
                    label: "Password",
                    placeholder: Some(""),
                    kind: ProviderSettingKind::Password,
                    required: false,
                    default_value: None,
                    hint: None,
                    is_advanced: false,
                },
            ];
            &SETTINGS
        }
        fn editor_language(&self) -> Arc<dyn EditorLanguage> {
            Arc::new(SqlLanguage::tsql())
        }
        fn create_connection(&self, _profile: ConnectionProfile) -> Box<dyn DatabaseConnection> {
            unimplemented!("not used in migration tests")
        }
    }

    fn test_catalog() -> ProviderCatalog {
        let mut catalog = ProviderCatalog::new();
        catalog.register(Arc::new(DummyMssqlProvider));
        catalog
    }

    /// Creates a realistic legacy Tauri SQLite database matching the exact schema from
    /// `src-tauri/src/storage/*.rs`.
    fn create_legacy_tauri_fixture(path: &Path) {
        let conn = Connection::open(path).expect("open legacy fixture");
        conn.execute_batch(
            "CREATE TABLE spaces (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                color TEXT NOT NULL DEFAULT '#6366f1',
                icon TEXT,
                connection_host TEXT,
                connection_port INTEGER DEFAULT 1433,
                connection_database TEXT,
                connection_username TEXT,
                connection_password TEXT,
                connection_trust_cert INTEGER DEFAULT 1,
                connection_encrypt INTEGER DEFAULT 0,
                mcp_access TEXT DEFAULT 'off',
                last_active_tab_id TEXT,
                connection_driver TEXT DEFAULT 'mssql',
                created_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now')),
                sort_order INTEGER DEFAULT 0
            );

            CREATE TABLE tab_folders (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                name TEXT NOT NULL,
                is_expanded INTEGER DEFAULT 1,
                sort_order INTEGER DEFAULT 0,
                created_at TEXT DEFAULT (datetime('now'))
            );

            CREATE TABLE pinned_tabs (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                title TEXT NOT NULL,
                tab_type TEXT NOT NULL DEFAULT 'query',
                content TEXT DEFAULT '',
                metadata TEXT,
                database TEXT,
                folder_id TEXT,
                is_pinned INTEGER DEFAULT 0,
                last_accessed_at TEXT DEFAULT (datetime('now')),
                created_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now')),
                sort_order INTEGER DEFAULT 0
            );

            CREATE TABLE archived_tabs (
                id TEXT PRIMARY KEY,
                original_tab_id TEXT,
                space_id TEXT NOT NULL,
                space_name TEXT,
                title TEXT NOT NULL,
                tab_type TEXT NOT NULL DEFAULT 'query',
                content TEXT DEFAULT '',
                metadata TEXT,
                database TEXT,
                was_pinned INTEGER DEFAULT 0,
                created_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now')),
                last_accessed_at TEXT DEFAULT (datetime('now')),
                archived_at TEXT DEFAULT (datetime('now'))
            );

            CREATE TABLE snippets (
                id TEXT PRIMARY KEY,
                trigger TEXT NOT NULL,
                name TEXT NOT NULL,
                content TEXT NOT NULL,
                description TEXT,
                is_builtin INTEGER DEFAULT 0,
                enabled INTEGER DEFAULT 1,
                category TEXT,
                drivers TEXT,
                created_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now'))
            );

            CREATE TABLE sticky_notes (
                id TEXT PRIMARY KEY,
                tab_id TEXT NOT NULL,
                line_number INTEGER NOT NULL,
                content TEXT NOT NULL,
                color TEXT DEFAULT 'yellow',
                minimized INTEGER DEFAULT 0,
                width INTEGER,
                pinned INTEGER DEFAULT 0,
                created_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now'))
            );

            CREATE TABLE app_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT DEFAULT (datetime('now'))
            );

            CREATE TABLE mcp_log (
                id TEXT PRIMARY KEY,
                ts TEXT NOT NULL,
                tool TEXT NOT NULL,
                connection_id TEXT,
                space_name TEXT,
                statement TEXT,
                access_level TEXT,
                status TEXT NOT NULL,
                duration_ms INTEGER DEFAULT 0,
                rows_returned INTEGER DEFAULT 0,
                decision_latency_ms INTEGER,
                decided_at TEXT
            );

            CREATE TABLE mcp_approvals (
                id TEXT PRIMARY KEY,
                ts TEXT NOT NULL,
                connection_id TEXT,
                space_name TEXT,
                driver TEXT,
                statement TEXT NOT NULL,
                access_level TEXT,
                status TEXT NOT NULL,
                decided_at TEXT
            );",
        )
        .expect("create legacy schema");

        // Spaces: one using keyring password, one using legacy SQLite fallback password.
        conn.execute(
            "INSERT INTO spaces (
                id, name, color, icon, connection_host, connection_port, connection_database,
                connection_username, connection_password, connection_trust_cert, connection_encrypt,
                mcp_access, last_active_tab_id, connection_driver, sort_order
             ) VALUES (
                'space-prod', 'Prod MSSQL', '#10B981', 'database', 'sql.prod.internal', 1433,
                'SalesDB', 'sa', NULL, 0, 1, 'read', 'tab-2', 'mssql', 0
             )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO spaces (
                id, name, color, icon, connection_host, connection_port, connection_database,
                connection_username, connection_password, connection_trust_cert, connection_encrypt,
                mcp_access, last_active_tab_id, connection_driver, sort_order
             ) VALUES (
                'space-pg', 'Analytics PG', '#6366F1', NULL, 'pg.local', 5432,
                'analytics', 'analyst', 'sqlite-fallback-pw', 1, 0, 'write', 'tab-pg-1', 'postgres', 1
             )",
            [],
        )
        .unwrap();

        // Folder
        conn.execute(
            "INSERT INTO tab_folders (id, space_id, name, is_expanded, sort_order)
             VALUES ('folder-reports', 'space-prod', 'Monthly Reports', 0, 0)",
            [],
        )
        .unwrap();

        // Tabs (including a pinned folder member, a loose pinned tab, an unpinned tab, and a non-query settings tab to ignore)
        conn.execute(
            "INSERT INTO pinned_tabs (
                id, space_id, title, tab_type, content, database, folder_id, is_pinned,
                last_accessed_at, sort_order
             ) VALUES (
                'tab-1', 'space-prod', 'Revenue', 'query',
                'SELECT * FROM dbo.orders;\nSELECT SUM(total) FROM dbo.invoices;',
                'SalesDB', 'folder-reports', 1, '2026-01-15 08:30:00', 0
             )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pinned_tabs (
                id, space_id, title, tab_type, content, database, folder_id, is_pinned,
                last_accessed_at, sort_order
             ) VALUES (
                'tab-2', 'space-prod', 'Query 7', 'query',
                'SELECT TOP 10 * FROM dbo.customers;',
                'SalesDB', NULL, 0, '2026-01-20 12:00:00', 1
             )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pinned_tabs (
                id, space_id, title, tab_type, content, database, folder_id, is_pinned,
                last_accessed_at, sort_order
             ) VALUES (
                'tab-settings', 'space-prod', 'Settings', 'settings', '', NULL, NULL, 0,
                '2026-01-20 12:00:00', 2
             )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pinned_tabs (
                id, space_id, title, tab_type, content, database, folder_id, is_pinned,
                last_accessed_at, sort_order
             ) VALUES (
                'tab-pg-1', 'space-pg', 'Daily Active Users', 'query',
                'SELECT count(*) FROM events;', 'analytics', NULL, 1,
                '2026-01-19 09:00:00', 0
             )",
            [],
        )
        .unwrap();

        // Archived tab
        conn.execute(
            "INSERT INTO archived_tabs (
                id, original_tab_id, space_id, space_name, title, tab_type, content,
                database, was_pinned, archived_at
             ) VALUES (
                'arch-row-1', 'orig-tab-99', 'space-prod', 'Prod MSSQL', 'Old Migration',
                'query', 'SELECT 1 AS archived_check;', 'SalesDB', 0, '2026-01-10 10:00:00'
             )",
            [],
        )
        .unwrap();

        // Snippets: one user snippet + one disabled legacy UUID built-in ('sel')
        conn.execute(
            "INSERT INTO snippets (
                id, trigger, name, content, description, is_builtin, enabled, category, drivers
             ) VALUES (
                'user-snip-1', 'mytop', 'My Top 5', 'SELECT TOP 5 * FROM ${cursor}',
                'Quick top 5', 0, 1, 'Select', 'mssql'
             )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snippets (
                id, trigger, name, content, description, is_builtin, enabled, category, drivers
             ) VALUES (
                'legacy-uuid-builtin-sel', 'sel', 'SELECT *', 'SELECT * FROM ${cursor}',
                'Select all columns from a table', 1, 0, 'Select', NULL
             )",
            [],
        )
        .unwrap();

        // Sticky notes: line-anchored on active tab-1 line 2, tab-level fallback when line out of bounds, and one on archived tab.
        conn.execute(
            "INSERT INTO sticky_notes (id, tab_id, line_number, content, created_at, updated_at)
             VALUES ('note-1', 'tab-1', 2, 'Check invoice index before running on prod', '2026-01-15 09:00:00', '2026-01-15 09:05:00')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sticky_notes (id, tab_id, line_number, content, created_at, updated_at)
             VALUES ('note-2', 'tab-1', 99, 'General query caveat', '2026-01-15 09:10:00', '2026-01-15 09:10:00')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sticky_notes (id, tab_id, line_number, content, created_at, updated_at)
             VALUES ('note-arch', 'orig-tab-99', 1, 'Archived tab note', '2026-01-10 09:00:00', '2026-01-10 09:00:00')",
            [],
        )
        .unwrap();

        // app_state settings
        for (k, v) in [
            ("last_space_id", "space-prod"),
            ("last_tab_id", "tab-2"),
            ("auto_archive_enabled", "false"),
            ("auto_archive_days", "45"),
            ("history_retention_days", "90"),
            ("mcp_enabled", "true"),
            ("theme", "light"),
        ] {
            conn.execute(
                "INSERT INTO app_state (key, value) VALUES (?1, ?2)",
                params![k, v],
            )
            .unwrap();
        }

        // Legacy mcp_log
        conn.execute(
            "INSERT INTO mcp_log (
                id, ts, tool, connection_id, space_name, statement, access_level,
                status, duration_ms, rows_returned
             ) VALUES (
                'log-1', '2026-01-18 14:22:00', 'run_query', 'space-prod', 'Prod MSSQL',
                'SELECT COUNT(*) FROM dbo.orders', 'read', 'ok', 42, 1
             )",
            [],
        )
        .unwrap();

        // Legacy mcp_approvals (one approved, one pending that must be swept to orphaned)
        conn.execute(
            "INSERT INTO mcp_approvals (
                id, ts, connection_id, space_name, driver, statement, access_level, status, decided_at
             ) VALUES (
                'appr-1', '2026-01-18 14:25:00', 'space-pg', 'Analytics PG', 'postgres',
                'UPDATE events SET processed = true', 'write', 'approved', '2026-01-18 14:25:12'
             )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO mcp_approvals (
                id, ts, connection_id, space_name, driver, statement, access_level, status, decided_at
             ) VALUES (
                'appr-2', '2026-01-18 14:30:00', 'space-pg', 'Analytics PG', 'postgres',
                'DELETE FROM temp_stage', 'write', 'pending', NULL
             )",
            [],
        )
        .unwrap();
    }

    fn assert_full_migration_state(
        store: &StudioStore,
        secrets: &Arc<InMemorySecretBackend>,
        catalog: &ProviderCatalog,
    ) {
        let state = store.load(catalog);
        assert_eq!(state.spaces.len(), 2);
        assert_eq!(state.active_space_id.as_deref(), Some("space-prod"));
        assert_eq!(state.theme, ThemePreference::Light);
        assert!(!state.auto_archive_enabled);
        assert_eq!(state.auto_archive_days, 45);
        assert_eq!(state.query_counter, 7);

        // Space 1: Prod MSSQL
        let prod = &state.spaces[0];
        assert_eq!(prod.id, "space-prod");
        assert_eq!(prod.name, "Prod MSSQL");
        assert_eq!(prod.color, "#10B981");
        assert_eq!(prod.icon.as_deref(), Some("database"));
        assert_eq!(prod.profile.provider_id, "mssql");
        assert_eq!(prod.profile.get("server"), Some("sql.prod.internal"));
        assert_eq!(prod.profile.get("port"), Some("1433"));
        assert_eq!(prod.profile.get("database"), Some("SalesDB"));
        assert_eq!(prod.profile.get("user"), Some("sa"));
        assert_eq!(prod.profile.get("encrypt"), Some("true"));
        assert_eq!(prod.profile.get("trustServerCertificate"), Some("false"));
        // Password hydrated from new secret target (migrated from legacy keyring).
        assert_eq!(prod.profile.get("password"), Some("keyring-prod-pw!"));
        assert_eq!(prod.active_tab_id.as_deref(), Some("tab-2"));
        // Settings tab filtered out -> only 2 query tabs remain.
        assert_eq!(prod.tabs.len(), 2);
        assert_eq!(prod.groups.len(), 1);
        assert_eq!(prod.groups[0].name, "Monthly Reports");
        assert!(!prod.groups[0].is_expanded);
        assert!(prod.groups[0].is_pinned); // inherited pin protection from member tab-1

        // Space 2: Analytics PG (password migrated from legacy SQLite column into secret backend)
        let pg = &state.spaces[1];
        assert_eq!(pg.id, "space-pg");
        assert_eq!(pg.profile.provider_id, "postgres");
        assert_eq!(
            secrets
                .get(&secret_target("space-pg", "password"))
                .unwrap()
                .as_deref(),
            Some("sqlite-fallback-pw")
        );

        // Ensure no plaintext passwords leaked into studio_snapshot JSON.
        let conn = Connection::open(store.db_path()).unwrap();
        let raw_json: String = conn
            .query_row("SELECT payload FROM studio_snapshot WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!raw_json.contains("keyring-prod-pw!"));
        assert!(!raw_json.contains("sqlite-fallback-pw"));

        // Archived tab preserved with original_tab_id so notes remain attached.
        assert_eq!(state.archived_tabs.len(), 1);
        assert_eq!(state.archived_tabs[0].id, "orig-tab-99");
        assert_eq!(state.archived_tabs[0].title, "Old Migration");
        assert!(state.archived_tabs[0].archived_at.contains('T'));

        // Snippets: user snippet preserved + built-in 'sel' kept disabled + NO duplicate built-ins.
        let snippets = store.load_snippets().unwrap();
        let user_snip = snippets.iter().find(|s| s.id == "user-snip-1").unwrap();
        assert_eq!(user_snip.trigger, "mytop");
        assert!(!user_snip.is_builtin);
        let sel_matches: Vec<_> = snippets
            .iter()
            .filter(|s| s.is_builtin && s.trigger == "sel")
            .collect();
        assert_eq!(sel_matches.len(), 1);
        assert_eq!(sel_matches[0].id, "builtin:all:sel");
        assert!(!sel_matches[0].is_enabled);

        // Notes: line-anchored note on line 2, tab-level fallback for out-of-range line, archived note linked.
        let tab1_notes = store.get_notes_for_tab("tab-1").unwrap();
        assert_eq!(tab1_notes.len(), 2);
        let anchored = tab1_notes.iter().find(|n| n.id == "note-1").unwrap();
        assert!(!anchored.anchor.is_tab);
        assert_eq!(anchored.anchor.line_hint, 2);
        assert_eq!(anchored.anchor.quote, "SELECT SUM(total) FROM dbo.invoices;");
        let fallback = tab1_notes.iter().find(|n| n.id == "note-2").unwrap();
        assert!(fallback.anchor.is_tab);

        let arch_notes = store.get_notes_for_tab("orig-tab-99").unwrap();
        assert_eq!(arch_notes.len(), 1);
        assert_eq!(arch_notes[0].body, "Archived tab note");

        // Meta & MCP settings
        assert_eq!(store.get_run_log_retention_days(), 90);
        assert!(store.is_mcp_enabled());
        assert_eq!(store.get_space_mcp_access("space-prod"), McpAccess::Read);
        assert_eq!(store.get_space_mcp_access("space-pg"), McpAccess::Write);

        // MCP log + run_log
        let mcp_log = store.get_mcp_log(50);
        assert_eq!(mcp_log.len(), 1);
        assert_eq!(mcp_log[0].tool, "run_query");
        assert_eq!(mcp_log[0].space_id.as_deref(), Some("space-prod"));
        assert_eq!(mcp_log[0].provider_id.as_deref(), Some("mssql"));
        assert_eq!(mcp_log[0].duration_ms, 42);

        let run_log = store.get_run_log(None, 50);
        assert_eq!(run_log.len(), 1);
        assert_eq!(run_log[0].actor, run_actor::AGENT);
        assert_eq!(run_log[0].status, run_status::OK);

        // MCP approvals (pending swept to orphaned)
        let appr1 = store.get_mcp_approval("appr-1").unwrap();
        assert_eq!(appr1.status, "approved");
        let appr2 = store.get_mcp_approval("appr-2").unwrap();
        assert_eq!(appr2.status, mcp_approval_status::ORPHANED);
        assert!(appr2.decided_at.is_some());

        // Migration marker recorded
        let marker: Option<String> = conn
            .query_row(
                "SELECT value FROM studio_meta WHERE key = ?1",
                params![TAURI_MIGRATION_META_KEY],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert!(marker.is_some());
    }

    #[test]
    fn migrates_external_legacy_tauri_db_creates_backup_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!(
            "larik_migration_ext_{}",
            uuid::Uuid::new_v4()
        ));
        let legacy_dir = dir.join("legacy-tauri");
        let new_dir = dir.join("new-studio");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::create_dir_all(&new_dir).unwrap();

        let legacy_db = legacy_dir.join("larik.db");
        create_legacy_tauri_fixture(&legacy_db);

        let secrets = Arc::new(InMemorySecretBackend::new());
        secrets
            .set(&legacy_tauri_secret_target("space-prod"), "keyring-prod-pw!")
            .unwrap();

        let target_db = new_dir.join("studio.db");
        let store = StudioStore::open_at_with_secrets_and_legacy(
            target_db.clone(),
            secrets.clone(),
            &[legacy_db.clone()],
        )
        .unwrap();

        let catalog = test_catalog();
        assert_full_migration_state(&store, &secrets, &catalog);

        // Backup was created alongside studio.db and original legacy DB remains intact.
        let backup_path = new_dir.join("larik.tauri-backup.db");
        assert!(backup_path.is_file());
        assert!(legacy_db.is_file());

        // Re-opening must be idempotent (no duplicate mcp_log/run_log/notes, no second backup).
        let store2 = StudioStore::open_at_with_secrets_and_legacy(
            target_db,
            secrets.clone(),
            &[legacy_db],
        )
        .unwrap();
        assert_full_migration_state(&store2, &secrets, &catalog);
        assert!(!new_dir.join("larik.tauri-backup-1.db").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrates_in_place_legacy_tauri_db_upgrades_schema_and_creates_backup() {
        let dir = std::env::temp_dir().join(format!(
            "larik_migration_inplace_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let db_path = dir.join("larik.db");
        create_legacy_tauri_fixture(&db_path);

        let secrets = Arc::new(InMemorySecretBackend::new());
        secrets
            .set(&legacy_tauri_secret_target("space-prod"), "keyring-prod-pw!")
            .unwrap();

        // Open directly on the legacy Tauri DB file (in-place upgrade).
        let store = StudioStore::open_at_with_secrets(db_path.clone(), secrets.clone()).unwrap();
        let catalog = test_catalog();
        assert_full_migration_state(&store, &secrets, &catalog);

        let backup_path = dir.join("larik.tauri-backup.db");
        assert!(backup_path.is_file());

        // Re-opening the same DB must be completely idempotent.
        let store2 = StudioStore::open_at_with_secrets(db_path, secrets.clone()).unwrap();
        assert_full_migration_state(&store2, &secrets, &catalog);
        assert!(!dir.join("larik.tauri-backup-1.db").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn does_not_clobber_existing_studio_database() {
        let dir = std::env::temp_dir().join(format!(
            "larik_migration_noclub_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let catalog = test_catalog();
        let secrets = Arc::new(InMemorySecretBackend::new());
        let target_db = dir.join("studio.db");

        // Initialize a fresh GPUI studio DB with its own saved state.
        let store = StudioStore::open_at_with_secrets(target_db.clone(), secrets.clone()).unwrap();
        let mut initial = StudioState::seed_default(&catalog);
        initial.spaces[0].name = "Existing GPUI Space".to_string();
        store.save(&initial, &catalog).unwrap();

        // Now place a legacy Tauri DB right beside it and re-open.
        let legacy_db = dir.join("larik.db");
        create_legacy_tauri_fixture(&legacy_db);

        let reopened = StudioStore::open_at_with_secrets_and_legacy(
            target_db,
            secrets,
            &[legacy_db],
        )
        .unwrap();
        let loaded = reopened.load(&catalog);
        assert_eq!(loaded.spaces[0].name, "Existing GPUI Space");
        assert!(!dir.join("larik.tauri-backup.db").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decodes_keyring_v3_utf16le_password_blob() {
        let password = "P@ssw0rd!—SQL";
        let utf16_bytes: Vec<u8> = password
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(decode_legacy_keyring_blob(&utf16_bytes).unwrap(), password);

        // Trailing NUL trimmed cleanly
        let mut with_nul = utf16_bytes.clone();
        with_nul.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(decode_legacy_keyring_blob(&with_nul).unwrap(), password);

        // Odd-length UTF-8 fallback
        assert_eq!(decode_legacy_keyring_blob(b"abc").unwrap(), "abc");
    }
}
