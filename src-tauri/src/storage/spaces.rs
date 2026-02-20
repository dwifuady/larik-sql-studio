// Spaces data model and storage operations
// Spaces are work environments that contain pinned tabs and ONE database connection (1:1)

use crate::db::traits::DatabaseType;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::{DatabaseManager, StorageResult};

/// A Space represents a work environment with exactly one database connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub database_type: Option<DatabaseType>,
    // Connection fields (1:1 - each space has exactly one connection)
    pub connection_host: Option<String>,
    pub connection_port: Option<i32>,
    pub connection_database: Option<String>,
    pub connection_username: Option<String>,
    #[serde(skip_serializing)] // Never send password to frontend
    pub connection_password: Option<String>,
    pub connection_trust_cert: bool,
    pub connection_encrypt: bool,
    pub postgres_sslmode: Option<String>,
    pub mysql_ssl_enabled: Option<bool>,
    pub last_active_tab_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub sort_order: i32,
}

impl Space {
    /// Check if this space has a connection configured
    pub fn has_connection(&self) -> bool {
        match self.database_type.as_ref() {
            Some(crate::db::traits::DatabaseType::Sqlite) => self.connection_database.is_some(),
            _ => self.connection_host.is_some() && self.connection_database.is_some(),
        }
    }
}

/// Input for creating a new space with its connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSpaceInput {
    pub name: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub database_type: Option<DatabaseType>,
    // Connection details
    pub connection_host: Option<String>,
    pub connection_port: Option<i32>,
    pub connection_database: Option<String>,
    pub connection_username: Option<String>,
    pub connection_password: Option<String>,
    pub connection_trust_cert: Option<bool>,
    pub connection_encrypt: Option<bool>,
    pub postgres_sslmode: Option<String>,
    pub mysql_ssl_enabled: Option<bool>,
}

/// Input for updating an existing space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSpaceInput {
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub database_type: Option<DatabaseType>,
    pub sort_order: Option<i32>,
    // Connection updates
    pub connection_host: Option<String>,
    pub connection_port: Option<i32>,
    pub connection_database: Option<String>,
    pub connection_username: Option<String>,
    pub connection_password: Option<String>,
    pub connection_trust_cert: Option<bool>,
    pub connection_encrypt: Option<bool>,
    pub postgres_sslmode: Option<String>,
    pub mysql_ssl_enabled: Option<bool>,
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

        // Serialize database_type
        let db_type_str = input
            .database_type
            .as_ref()
            .map(|t| serde_json::to_string(t).unwrap_or_default());

        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO spaces (
                    id, name, color, icon, sort_order,
                    connection_host, connection_port, connection_database,
                    connection_username, connection_password,
                    connection_trust_cert, connection_encrypt,
                    database_type, postgres_sslmode, mysql_ssl_enabled,
                    created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'), datetime('now'))
                "#,
                params![
                    id,
                    input.name,
                    input.color,
                    input.icon,
                    sort_order,
                    input.connection_host,
                    input.connection_port.unwrap_or(1433),
                    input.connection_database,
                    input.connection_username,
                    input.connection_password,
                    input.connection_trust_cert.unwrap_or(true),
                    input.connection_encrypt.unwrap_or(false),
                    db_type_str,
                    input.postgres_sslmode,
                    input.mysql_ssl_enabled.map(|b| if b { 1 } else { 0 }),
                ],
            )?;
            Ok(())
        })?;

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
                    database_type, postgres_sslmode, mysql_ssl_enabled
                FROM spaces WHERE id = ?1"#,
            )?;

            let result = stmt.query_row(params![id], |row| {
                let db_type_str: Option<String> = row.get(15)?;
                let db_type = db_type_str.and_then(|s| serde_json::from_str(&s).ok());

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
                    database_type: db_type,
                    postgres_sslmode: row.get(16)?,
                    mysql_ssl_enabled: row.get::<_, Option<i32>>(17)?.map(|v| v != 0),
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
                    database_type, postgres_sslmode, mysql_ssl_enabled
                FROM spaces ORDER BY sort_order"#,
            )?;

            let spaces = stmt
                .query_map([], |row| {
                    let db_type_str: Option<String> = row.get(15)?;
                    let db_type = db_type_str.and_then(|s| serde_json::from_str(&s).ok());

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
                        database_type: db_type,
                        postgres_sslmode: row.get(16)?,
                        mysql_ssl_enabled: row.get::<_, Option<i32>>(17)?.map(|v| v != 0),
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

            // Database Type
            if let Some(ref db_type) = input.database_type {
                updates.push("database_type = ?");
                let s = serde_json::to_string(db_type).unwrap_or_default();
                params_vec.push(Box::new(s));
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
            if let Some(ref password) = input.connection_password {
                updates.push("connection_password = ?");
                params_vec.push(Box::new(password.clone()));
            }
            if let Some(trust_cert) = input.connection_trust_cert {
                updates.push("connection_trust_cert = ?");
                params_vec.push(Box::new(trust_cert as i32));
            }
            if let Some(encrypt) = input.connection_encrypt {
                updates.push("connection_encrypt = ?");
                params_vec.push(Box::new(encrypt as i32));
            }

            // New SSL fields
            if let Some(ref sslmode) = input.postgres_sslmode {
                updates.push("postgres_sslmode = ?");
                params_vec.push(Box::new(sslmode.clone()));
            }
            if let Some(mysql_ssl) = input.mysql_ssl_enabled {
                updates.push("mysql_ssl_enabled = ?");
                params_vec.push(Box::new(if mysql_ssl { 1 } else { 0 }));
            }

            params_vec.push(Box::new(id.to_string()));

            let sql = format!("UPDATE spaces SET {} WHERE id = ?", updates.join(", "));

            conn.execute(
                &sql,
                rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            )?;
            Ok(())
        })?;

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

    /// Get the connection password for a space (separate method to keep it secure)
    pub fn get_space_password(&self, id: &str) -> StorageResult<Option<String>> {
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
        self.with_connection(|conn| {
            let rows_affected = conn.execute("DELETE FROM spaces WHERE id = ?1", params![id])?;
            Ok(rows_affected > 0)
        })
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
}
