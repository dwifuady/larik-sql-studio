// Spaces data model and storage operations
// Spaces are work environments that contain pinned tabs and ONE database connection (1:1)

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::{DatabaseManager, StorageResult};
use crate::db::DriverKind;

/// OS keychain service name for saved space passwords (Windows Credential
/// Manager). Secrets live here — the SQLite column is kept empty after the
/// startup migration.
const KEYRING_SERVICE: &str = "larik-sql-studio";

/// Keychain entry for a space password. Only Windows has a real backend
/// compiled in (`keyring` feature `windows-native`); elsewhere the crate falls
/// back to an in-memory mock whose secrets vanish between entries, so every
/// other platform reports the keychain as unavailable and keeps passwords in
/// SQLite.
fn keyring_entry(space_id: &str) -> Result<keyring::Entry, keyring::Error> {
    if cfg!(windows) {
        keyring::Entry::new(KEYRING_SERVICE, &format!("space-password/{space_id}"))
    } else {
        Err(keyring::Error::NoStorageAccess(
            "no OS keychain backend on this platform".into(),
        ))
    }
}

/// Store a space password in the OS keychain and clear the SQLite column.
/// An empty password clears both. Any keychain failure keeps the value in
/// SQLite (previous behavior) rather than losing it.
fn store_password_in_keyring(
    manager: &DatabaseManager,
    space_id: &str,
    password: &str,
) -> StorageResult<()> {
    let entry = keyring_entry(space_id);
    if !password.is_empty() {
        match entry.as_ref().map(|e| e.set_password(password)) {
            Ok(Ok(())) => {
                return manager.with_connection(|conn| {
                    conn.execute(
                        "UPDATE spaces SET connection_password = NULL WHERE id = ?1",
                        params![space_id],
                    )?;
                    Ok(())
                });
            }
            Ok(Err(e)) => eprintln!("[storage] keychain write failed, keeping SQLite copy: {e}"),
            Err(e) => eprintln!("[storage] keychain unavailable, keeping SQLite copy: {e}"),
        }
    }
    // Empty password, or the keychain write failed: an older keychain secret
    // would still win in `get_space_password`, so drop it.
    if let Ok(entry) = &entry {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => eprintln!("[storage] keychain delete failed: {e}"),
        }
    }
    let stored = (!password.is_empty()).then_some(password);
    manager.with_connection(|conn| {
        conn.execute(
            "UPDATE spaces SET connection_password = ?1 WHERE id = ?2",
            params![stored, space_id],
        )?;
        Ok(())
    })
}

/// One-time migration: move plaintext passwords from SQLite into the OS
/// keychain. Rows whose keychain write fails keep their value and are retried
/// on the next startup. Returns the number of migrated rows.
pub fn migrate_space_passwords_to_keyring(manager: &DatabaseManager) -> StorageResult<usize> {
    // No keychain backend off Windows (see `keyring_entry`): nothing to move,
    // and no point logging a failure per row on every startup.
    if !cfg!(windows) {
        return Ok(0);
    }
    let rows: Vec<(String, String)> = manager.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, connection_password FROM spaces \
             WHERE connection_password IS NOT NULL AND connection_password != ''",
        )?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    })?;

    let mut migrated = 0;
    for (id, password) in rows {
        match keyring_entry(&id) {
            Ok(entry) => match entry.set_password(&password) {
                Ok(()) => {
                    manager.with_connection(|conn| {
                        conn.execute(
                            "UPDATE spaces SET connection_password = NULL WHERE id = ?1",
                            params![id],
                        )?;
                        Ok(())
                    })?;
                    migrated += 1;
                }
                Err(e) => eprintln!("[storage] keychain migration failed for space {id}: {e}"),
            },
            Err(e) => eprintln!("[storage] keychain unavailable for space {id}: {e}"),
        }
    }
    Ok(migrated)
}

/// A Space represents a work environment with exactly one database connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    // Connection fields (1:1 - each space has exactly one connection)
    pub connection_host: Option<String>,
    pub connection_port: Option<i32>,
    pub connection_database: Option<String>,
    pub connection_username: Option<String>,
    #[serde(skip_serializing)] // Never send password to frontend
    pub connection_password: Option<String>,
    pub connection_trust_cert: bool,
    pub connection_encrypt: bool,
    /// Database driver kind (`DriverKind` as a string, e.g. "mssql"). Kept as a raw
    /// string so a space saved by a newer build still loads; it is parsed on connect.
    pub connection_driver: String,
    /// Exposure level for the embedded MCP server: `"off"`, `"read"`, or `"write"`.
    pub mcp_access: String,
    pub last_active_tab_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub sort_order: i32,
}
impl Space {
    /// Check if this space has a connection configured.
    /// MongoDB lists its databases once connected, so its database is optional.
    pub fn has_connection(&self) -> bool {
        let filled = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.trim().is_empty());
        filled(&self.connection_host)
            && (filled(&self.connection_database)
                || self.connection_driver == DriverKind::Mongodb.as_str())
    }
}

/// Input for creating a new space with its connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSpaceInput {
    pub name: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    // Connection details
    pub connection_host: Option<String>,
    pub connection_port: Option<i32>,
    pub connection_database: Option<String>,
    pub connection_username: Option<String>,
    pub connection_password: Option<String>,
    pub connection_trust_cert: Option<bool>,
    pub connection_encrypt: Option<bool>,
    pub connection_driver: Option<DriverKind>,
    pub mcp_access: Option<String>,
}

/// Input for updating an existing space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSpaceInput {
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
    // Connection updates
    pub connection_host: Option<String>,
    pub connection_port: Option<i32>,
    pub connection_database: Option<String>,
    pub connection_username: Option<String>,
    pub connection_password: Option<String>,
    pub connection_trust_cert: Option<bool>,
    pub connection_encrypt: Option<bool>,
    pub connection_driver: Option<DriverKind>,
    pub mcp_access: Option<String>,
}

/// Normalize a requested MCP access level to a known value; unknown → `"off"`.
fn normalize_access(v: Option<&str>) -> &'static str {
    match v {
        Some("read") => "read",
        Some("write") => "write",
        _ => "off",
    }
}

/// Sanitize an access level read from storage; unknown → `"off"` (fail-closed).
fn sanitize_access(v: String) -> String {
    match v.as_str() {
        "read" | "write" => v,
        _ => "off".to_string(),
    }
}

impl DatabaseManager {
    /// Create a new space with optional connection
    pub fn create_space(&self, input: CreateSpaceInput) -> StorageResult<Space> {
        let id = Uuid::new_v4().to_string();

        // Get the next sort order
        let max_order: i32 = self.with_connection(|conn| {
            conn.query_row(
                "SELECT COALESCE(MAX(sort_order), -1) FROM spaces",
                [],
                |row| row.get(0),
            )
        })?;
        let sort_order = max_order + 1;
        let driver = input.connection_driver.unwrap_or_default();

        let mcp_access = normalize_access(input.mcp_access.as_deref());
        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO spaces (
                    id, name, color, icon, sort_order,
                    connection_host, connection_port, connection_database,
                    connection_username, connection_password,
                    connection_trust_cert, connection_encrypt, connection_driver,
                    mcp_access,
                    created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now'), datetime('now'))
                "#,
                params![
                    id,
                    input.name,
                    input.color,
                    input.icon,
                    sort_order,
                    input.connection_host,
                    input.connection_port.unwrap_or(i32::from(driver.default_port())),
                    input.connection_database,
                    input.connection_username,
                    None::<String>, // password lives in the OS keychain; see store_password_in_keyring
                    input.connection_trust_cert.unwrap_or(true),
                    input.connection_encrypt.unwrap_or(false),
                    driver.as_str(),
                    mcp_access
                ],
            )?;
            Ok(())
        })?;

        // Route the password into the OS keychain (column stays empty).
        if let Some(password) = &input.connection_password {
            store_password_in_keyring(self, &id, password)?;
        }

        // Fetch and return the created space
        self.get_space(&id)?.ok_or_else(|| {
            super::database::StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows)
        })
    }

    /// Get a space by ID
    pub fn get_space(&self, id: &str) -> StorageResult<Option<Space>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT
                    id, name, color, icon,
                    connection_host, connection_port, connection_database,
                    connection_username, connection_password,
                    connection_trust_cert, connection_encrypt,
                    last_active_tab_id,
                    created_at, updated_at, sort_order,
                    connection_driver, mcp_access
                FROM spaces WHERE id = ?1"#,
            )?;

            let result = stmt.query_row(params![id], |row| {
                Ok(Space {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                    icon: row.get(3)?,
                    connection_host: row.get(4)?,
                    connection_port: row.get(5)?,
                    connection_database: row.get(6)?,
                    connection_username: row.get(7)?,
                    connection_password: row.get(8)?,
                    connection_trust_cert: row.get::<_, Option<i32>>(9)?.unwrap_or(1) != 0,
                    connection_encrypt: row.get::<_, Option<i32>>(10)?.unwrap_or(0) != 0,
                    last_active_tab_id: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                    sort_order: row.get(14)?,
                    connection_driver: row.get(15)?,
                    mcp_access: sanitize_access(row.get(16)?),
                })
            });

            match result {
                Ok(space) => Ok(Some(space)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Get all spaces ordered by sort_order
    pub fn get_all_spaces(&self) -> StorageResult<Vec<Space>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT
                    id, name, color, icon,
                    connection_host, connection_port, connection_database,
                    connection_username, connection_password,
                    connection_trust_cert, connection_encrypt,
                    last_active_tab_id,
                    created_at, updated_at, sort_order,
                    connection_driver, mcp_access
                FROM spaces ORDER BY sort_order"#,
            )?;

            let spaces = stmt
                .query_map([], |row| {
                    Ok(Space {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        color: row.get(2)?,
                        icon: row.get(3)?,
                        connection_host: row.get(4)?,
                        connection_port: row.get(5)?,
                        connection_database: row.get(6)?,
                        connection_username: row.get(7)?,
                        connection_password: row.get(8)?,
                        connection_trust_cert: row.get::<_, Option<i32>>(9)?.unwrap_or(1) != 0,
                        connection_encrypt: row.get::<_, Option<i32>>(10)?.unwrap_or(0) != 0,
                        last_active_tab_id: row.get(11)?,
                        created_at: row.get(12)?,
                        updated_at: row.get(13)?,
                        sort_order: row.get(14)?,
                        connection_driver: row.get(15)?,
                        mcp_access: sanitize_access(row.get(16)?),
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(spaces)
        })
    }

    /// Update an existing space
    pub fn update_space(&self, id: &str, input: UpdateSpaceInput) -> StorageResult<Option<Space>> {
        // Check if space exists
        if self.get_space(id)?.is_none() {
            return Ok(None);
        }

        self.with_connection(|conn| {
            let mut updates = vec!["updated_at = datetime('now')"];
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![];

            if let Some(ref name) = input.name {
                updates.push("name = ?");
                params_vec.push(Box::new(name.clone()));
            }
            if let Some(ref color) = input.color {
                updates.push("color = ?");
                params_vec.push(Box::new(color.clone()));
            }
            if let Some(ref icon) = input.icon {
                updates.push("icon = ?");
                params_vec.push(Box::new(icon.clone()));
            }
            if let Some(sort_order) = input.sort_order {
                updates.push("sort_order = ?");
                params_vec.push(Box::new(sort_order));
            }
            // Connection fields
            if let Some(ref host) = input.connection_host {
                updates.push("connection_host = ?");
                params_vec.push(Box::new(host.clone()));
            }
            if let Some(port) = input.connection_port {
                updates.push("connection_port = ?");
                params_vec.push(Box::new(port));
            }
            if let Some(ref database) = input.connection_database {
                updates.push("connection_database = ?");
                params_vec.push(Box::new(database.clone()));
            }
            if let Some(ref username) = input.connection_username {
                updates.push("connection_username = ?");
                params_vec.push(Box::new(username.clone()));
            }
            // Password updates go to the OS keychain, not the SQLite column.
            // (Handled after the UPDATE below.)
            if let Some(trust_cert) = input.connection_trust_cert {
                updates.push("connection_trust_cert = ?");
                params_vec.push(Box::new(trust_cert as i32));
            }
            if let Some(encrypt) = input.connection_encrypt {
                updates.push("connection_encrypt = ?");
                params_vec.push(Box::new(encrypt as i32));
            }
            if let Some(driver) = input.connection_driver {
                updates.push("connection_driver = ?");
                params_vec.push(Box::new(driver.as_str()));
            }
            if let Some(mcp_access) = &input.mcp_access {
                updates.push("mcp_access = ?");
                params_vec.push(Box::new(
                    normalize_access(Some(mcp_access.as_str())).to_string(),
                ));
            }
            params_vec.push(Box::new(id.to_string()));

            let sql = format!("UPDATE spaces SET {} WHERE id = ?", updates.join(", "));

            conn.execute(
                &sql,
                rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            )?;
            Ok(())
        })?;

        if let Some(password) = &input.connection_password {
            store_password_in_keyring(self, id, password)?;
        }

        self.get_space(id)
    }

    /// Update the last active tab ID for a space
    pub fn update_space_last_active_tab(
        &self,
        space_id: &str,
        tab_id: Option<&str>,
    ) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                "UPDATE spaces SET last_active_tab_id = ?, updated_at = datetime('now') WHERE id = ?",
                params![tab_id, space_id],
            )?;
            Ok(())
        })
    }

    /// Get the connection password for a space (separate method to keep it
    /// secure). The OS keychain is authoritative; the SQLite column only
    /// holds values that never migrated or whose keychain write failed.
    pub fn get_space_password(&self, id: &str) -> StorageResult<Option<String>> {
        if let Ok(entry) = keyring_entry(id) {
            match entry.get_password() {
                Ok(password) => return Ok(Some(password)),
                Err(keyring::Error::NoEntry) => {}
                Err(e) => eprintln!("[storage] keychain read failed, using SQLite: {e}"),
            }
        }
        self.with_connection(|conn| {
            let result = conn.query_row(
                "SELECT connection_password FROM spaces WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<String>>(0),
            );

            match result {
                Ok(pwd) => Ok(pwd),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Delete a space by ID (cascades to pinned_tabs)
    pub fn delete_space(&self, id: &str) -> StorageResult<bool> {
        // Drop the keychain secret too — orphaned credentials accumulate otherwise.
        if let Ok(entry) = keyring_entry(id) {
            let _ = entry.delete_credential();
        }
        let rows_affected = self.with_connection(|conn| {
            let rows = conn.execute("DELETE FROM spaces WHERE id = ?1", params![id])?;
            Ok(rows)
        })?;
        Ok(rows_affected > 0)
    }

    /// Reorder spaces - update sort_order for a list of space IDs
    pub fn reorder_spaces(&self, space_ids: &[String]) -> StorageResult<()> {
        self.with_connection(|conn| {
            for (index, id) in space_ids.iter().enumerate() {
                conn.execute(
                    "UPDATE spaces SET sort_order = ?1, updated_at = datetime('now') WHERE id = ?2",
                    params![index as i32, id],
                )?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn create_test_db() -> (DatabaseManager, PathBuf) {
        let temp_dir = std::env::temp_dir();
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let db_path = temp_dir.join(format!(
            "larik_spaces_test_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&db_path);
        let manager = DatabaseManager::new(db_path.clone()).unwrap();
        (manager, db_path)
    }

    #[test]
    fn test_create_space() {
        let (manager, db_path) = create_test_db();

        let space = manager
            .create_space(CreateSpaceInput {
                name: "Test Space".to_string(),
                color: Some("#FF5733".to_string()),
                icon: None,
                connection_host: Some("localhost".to_string()),
                connection_port: Some(1433),
                connection_database: Some("testdb".to_string()),
                connection_username: Some("sa".to_string()),
                connection_password: Some("password".to_string()),
                connection_trust_cert: Some(true),
                connection_encrypt: Some(false),
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        assert_eq!(space.name, "Test Space");
        assert_eq!(space.color, Some("#FF5733".to_string()));
        assert_eq!(space.sort_order, 0);
        assert!(!space.id.is_empty());
        assert_eq!(space.connection_host, Some("localhost".to_string()));
        assert_eq!(space.connection_database, Some("testdb".to_string()));
        assert!(space.has_connection());
        // Spaces default to the SQL Server driver
        assert_eq!(space.connection_driver, "mssql");

        let _ = std::fs::remove_file(&db_path);
    }

    /// Whether this machine has a usable OS keychain. CI runners (headless
    /// Linux, no Secret Service) don't, and every write takes the SQLite
    /// fallback there.
    fn keychain_available() -> bool {
        let Ok(entry) = keyring_entry(&format!("probe-{}", Uuid::new_v4())) else {
            return false;
        };
        let ok = entry.set_password("probe").is_ok() && entry.get_password().is_ok();
        let _ = entry.delete_credential();
        ok
    }

    /// Passwords live in the OS keychain when there is one: readable through
    /// `get_space_password`, absent from the SQLite column, and gone when the
    /// space is deleted. Without a keychain the SQLite fallback must hold the
    /// password instead of losing it.
    #[test]
    fn password_lives_in_keychain_not_sqlite() {
        let (manager, db_path) = create_test_db();

        let space = manager
            .create_space(CreateSpaceInput {
                name: "Keychain Space".to_string(),
                color: None,
                icon: None,
                connection_host: Some("localhost".to_string()),
                connection_port: Some(1433),
                connection_database: Some("db".to_string()),
                connection_username: Some("sa".to_string()),
                connection_password: Some("s3cret".to_string()),
                connection_trust_cert: Some(true),
                connection_encrypt: Some(false),
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        assert_eq!(
            manager.get_space_password(&space.id).unwrap().as_deref(),
            Some("s3cret")
        );

        // The SQLite column is empty when the keychain holds the secret, and
        // holds it otherwise.
        let stored: Option<String> = manager
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT connection_password FROM spaces WHERE id = ?1",
                    params![&space.id],
                    |row| row.get(0),
                )
            })
            .unwrap();
        let expected = (!keychain_available()).then(|| "s3cret".to_string());
        assert_eq!(stored, expected);

        // Deleting the space removes the keychain secret.
        assert!(manager.delete_space(&space.id).unwrap());
        assert_eq!(manager.get_space_password(&space.id).unwrap(), None);

        let _ = std::fs::remove_file(&db_path);
    }

    /// An empty password clears the saved one everywhere: the keychain entry
    /// must go too, or `get_space_password` keeps returning the old secret.
    #[test]
    fn empty_password_clears_keychain_and_sqlite() {
        let (manager, db_path) = create_test_db();

        let space = manager
            .create_space(CreateSpaceInput {
                name: "Clear Space".to_string(),
                color: None,
                icon: None,
                connection_host: Some("localhost".to_string()),
                connection_port: Some(1433),
                connection_database: Some("db".to_string()),
                connection_username: Some("sa".to_string()),
                connection_password: Some("old-secret".to_string()),
                connection_trust_cert: None,
                connection_encrypt: None,
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        manager
            .update_space(
                &space.id,
                UpdateSpaceInput {
                    name: None,
                    color: None,
                    icon: None,
                    sort_order: None,
                    connection_host: None,
                    connection_port: None,
                    connection_database: None,
                    connection_username: None,
                    connection_password: Some(String::new()),
                    connection_trust_cert: None,
                    connection_encrypt: None,
                    connection_driver: None,
                    mcp_access: None,
                },
            )
            .unwrap();

        assert_eq!(manager.get_space_password(&space.id).unwrap(), None);

        manager.delete_space(&space.id).unwrap();
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_get_space() {
        let (manager, db_path) = create_test_db();

        let created = manager
            .create_space(CreateSpaceInput {
                name: "My Space".to_string(),
                color: None,
                icon: Some("🚀".to_string()),
                connection_host: None,
                connection_port: None,
                connection_database: None,
                connection_username: None,
                connection_password: None,
                connection_trust_cert: None,
                connection_encrypt: None,
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        let fetched = manager.get_space(&created.id).unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "My Space");
        assert_eq!(fetched.icon, Some("🚀".to_string()));
        assert!(!fetched.has_connection());

        // A host alone is a connection only for MongoDB
        let mut host_only = fetched.clone();
        host_only.connection_host = Some("192.168.0.10".to_string());
        host_only.connection_database = Some(String::new());
        assert!(!host_only.has_connection());
        host_only.connection_driver = "mongodb".to_string();
        assert!(host_only.has_connection());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_get_nonexistent_space() {
        let (manager, db_path) = create_test_db();

        let result = manager.get_space("nonexistent-id").unwrap();
        assert!(result.is_none());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_get_all_spaces() {
        let (manager, db_path) = create_test_db();

        manager
            .create_space(CreateSpaceInput {
                name: "Space A".to_string(),
                color: None,
                icon: None,
                connection_host: None,
                connection_port: None,
                connection_database: None,
                connection_username: None,
                connection_password: None,
                connection_trust_cert: None,
                connection_encrypt: None,
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        manager
            .create_space(CreateSpaceInput {
                name: "Space B".to_string(),
                color: None,
                icon: None,
                connection_host: None,
                connection_port: None,
                connection_database: None,
                connection_username: None,
                connection_password: None,
                connection_trust_cert: None,
                connection_encrypt: None,
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        let spaces = manager.get_all_spaces().unwrap();
        assert_eq!(spaces.len(), 2);
        assert_eq!(spaces[0].name, "Space A");
        assert_eq!(spaces[1].name, "Space B");
        assert_eq!(spaces[0].sort_order, 0);
        assert_eq!(spaces[1].sort_order, 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_update_space() {
        let (manager, db_path) = create_test_db();

        let created = manager
            .create_space(CreateSpaceInput {
                name: "Original".to_string(),
                color: None,
                icon: None,
                connection_host: None,
                connection_port: None,
                connection_database: None,
                connection_username: None,
                connection_password: None,
                connection_trust_cert: None,
                connection_encrypt: None,
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        let updated = manager
            .update_space(
                &created.id,
                UpdateSpaceInput {
                    name: Some("Updated".to_string()),
                    color: Some("#00FF00".to_string()),
                    icon: None,
                    sort_order: None,
                    connection_host: Some("localhost".to_string()),
                    connection_port: Some(1433),
                    connection_database: Some("mydb".to_string()),
                    connection_username: Some("user".to_string()),
                    connection_password: Some("pass".to_string()),
                    connection_trust_cert: Some(true),
                    connection_encrypt: Some(false),
                    connection_driver: Some(DriverKind::Mssql),
                    mcp_access: None,
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(updated.name, "Updated");
        assert_eq!(updated.color, Some("#00FF00".to_string()));
        assert_eq!(updated.connection_host, Some("localhost".to_string()));
        assert!(updated.has_connection());
        assert_eq!(updated.connection_driver, "mssql");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_delete_space() {
        let (manager, db_path) = create_test_db();

        let space = manager
            .create_space(CreateSpaceInput {
                name: "To Delete".to_string(),
                color: None,
                icon: None,
                connection_host: None,
                connection_port: None,
                connection_database: None,
                connection_username: None,
                connection_password: None,
                connection_trust_cert: None,
                connection_encrypt: None,
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        let deleted = manager.delete_space(&space.id).unwrap();
        assert!(deleted);

        let fetched = manager.get_space(&space.id).unwrap();
        assert!(fetched.is_none());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_reorder_spaces() {
        let (manager, db_path) = create_test_db();

        let space_a = manager
            .create_space(CreateSpaceInput {
                name: "A".to_string(),
                color: None,
                icon: None,
                connection_host: None,
                connection_port: None,
                connection_database: None,
                connection_username: None,
                connection_password: None,
                connection_trust_cert: None,
                connection_encrypt: None,
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        let space_b = manager
            .create_space(CreateSpaceInput {
                name: "B".to_string(),
                color: None,
                icon: None,
                connection_host: None,
                connection_port: None,
                connection_database: None,
                connection_username: None,
                connection_password: None,
                connection_trust_cert: None,
                connection_encrypt: None,
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        let space_c = manager
            .create_space(CreateSpaceInput {
                name: "C".to_string(),
                color: None,
                icon: None,
                connection_host: None,
                connection_port: None,
                connection_database: None,
                connection_username: None,
                connection_password: None,
                connection_trust_cert: None,
                connection_encrypt: None,
                connection_driver: None,
                mcp_access: None,
            })
            .unwrap();

        // Reorder: C, A, B
        manager
            .reorder_spaces(&[space_c.id.clone(), space_a.id.clone(), space_b.id.clone()])
            .unwrap();

        let spaces = manager.get_all_spaces().unwrap();
        assert_eq!(spaces[0].name, "C");
        assert_eq!(spaces[1].name, "A");
        assert_eq!(spaces[2].name, "B");

        let _ = std::fs::remove_file(&db_path);
    }
}
