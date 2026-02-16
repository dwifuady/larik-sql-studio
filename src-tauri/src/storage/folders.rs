// Tab folders data model and storage operations
// Folders organize pinned tabs within spaces (Arc Browser-style)

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::{DatabaseManager, StorageResult};
use super::tabs::Tab;

/// A TabFolder represents a folder containing pinned tabs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabFolder {
    pub id: String,
    pub space_id: String,
    pub name: String,
    pub is_expanded: bool,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating a new folder
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFolderInput {
    pub space_id: String,
    pub name: String,
}

/// Input for updating an existing folder
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateFolderInput {
    pub name: Option<String>,
    pub is_expanded: Option<bool>,
    pub sort_order: Option<i32>,
}

impl DatabaseManager {
    /// Create a new folder
    pub fn create_folder(&self, input: CreateFolderInput) -> StorageResult<TabFolder> {
        let id = Uuid::new_v4().to_string();

        // Get the minimum sort order for folders in this space (new folders appear at top)
        let min_order: i32 = self.with_connection(|conn| {
            conn.query_row(
                "SELECT COALESCE(MIN(sort_order), 1) FROM tab_folders WHERE space_id = ?1",
                params![input.space_id],
                |row| row.get(0),
            )
        })?;
        let sort_order = min_order - 1;

        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO tab_folders (id, space_id, name, is_expanded, sort_order, created_at, updated_at)
                VALUES (?1, ?2, ?3, 1, ?4, datetime('now'), datetime('now'))
                "#,
                params![id, input.space_id, input.name, sort_order],
            )?;
            Ok(())
        })?;

        self.get_folder(&id)?.ok_or_else(|| {
            super::database::StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows)
        })
    }

    /// Get a folder by ID
    pub fn get_folder(&self, id: &str) -> StorageResult<Option<TabFolder>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, space_id, name, is_expanded, sort_order, created_at, updated_at
                 FROM tab_folders WHERE id = ?1",
            )?;

            let result = stmt.query_row(params![id], |row| {
                let is_expanded_int: i32 = row.get(3)?;
                Ok(TabFolder {
                    id: row.get(0)?,
                    space_id: row.get(1)?,
                    name: row.get(2)?,
                    is_expanded: is_expanded_int != 0,
                    sort_order: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            });

            match result {
                Ok(folder) => Ok(Some(folder)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Get all folders for a space, ordered by sort_order
    pub fn get_folders_by_space(&self, space_id: &str) -> StorageResult<Vec<TabFolder>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, space_id, name, is_expanded, sort_order, created_at, updated_at
                 FROM tab_folders WHERE space_id = ?1 ORDER BY sort_order",
            )?;

            let folders = stmt
                .query_map(params![space_id], |row| {
                    let is_expanded_int: i32 = row.get(3)?;
                    Ok(TabFolder {
                        id: row.get(0)?,
                        space_id: row.get(1)?,
                        name: row.get(2)?,
                        is_expanded: is_expanded_int != 0,
                        sort_order: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(folders)
        })
    }

    /// Update an existing folder
    pub fn update_folder(
        &self,
        id: &str,
        input: UpdateFolderInput,
    ) -> StorageResult<Option<TabFolder>> {
        // Check if folder exists
        if self.get_folder(id)?.is_none() {
            return Ok(None);
        }

        self.with_connection(|conn| {
            let mut updates = vec!["updated_at = datetime('now')"];
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![];

            if let Some(ref name) = input.name {
                updates.push("name = ?");
                params_vec.push(Box::new(name.clone()));
            }
            if let Some(is_expanded) = input.is_expanded {
                updates.push("is_expanded = ?");
                params_vec.push(Box::new(is_expanded as i32));
            }
            if let Some(sort_order) = input.sort_order {
                updates.push("sort_order = ?");
                params_vec.push(Box::new(sort_order));
            }

            params_vec.push(Box::new(id.to_string()));

            let sql = format!("UPDATE tab_folders SET {} WHERE id = ?", updates.join(", "));

            conn.execute(
                &sql,
                rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            )?;
            Ok(())
        })?;

        self.get_folder(id)
    }

    /// Delete a folder by ID (tabs in folder become ungrouped)
    pub fn delete_folder(&self, id: &str) -> StorageResult<bool> {
        // Get all tab IDs in this folder first
        let tab_ids: Vec<String> = self.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id FROM pinned_tabs WHERE folder_id = ?1")?;
            let ids = stmt
                .query_map(params![id], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ids)
        })?;

        // Archive all tabs in the folder
        for tab_id in tab_ids {
            self.archive_tab(&tab_id)?;
        }

        // Delete the folder
        self.with_connection(|conn| {
            let rows_affected =
                conn.execute("DELETE FROM tab_folders WHERE id = ?1", params![id])?;
            Ok(rows_affected > 0)
        })
    }

    /// Add a tab to a folder (also ensures the tab is pinned)
    pub fn add_tab_to_folder(&self, tab_id: &str, folder_id: &str) -> StorageResult<bool> {
        self.with_connection(|conn| {
            // Verify folder exists and get its space_id
            let folder_space_id: Option<String> = conn.query_row(
                "SELECT space_id FROM tab_folders WHERE id = ?1",
                params![folder_id],
                |row| row.get(0),
            ).ok();

            if folder_space_id.is_none() {
                return Ok(false);
            }

            // Update tab: set folder_id and ensure it's pinned
            let rows_affected = conn.execute(
                "UPDATE pinned_tabs SET folder_id = ?1, is_pinned = 1, updated_at = datetime('now') WHERE id = ?2",
                params![folder_id, tab_id],
            )?;
            Ok(rows_affected > 0)
        })
    }

    /// Remove a tab from its folder (sets folder_id to NULL)
    pub fn remove_tab_from_folder(&self, tab_id: &str) -> StorageResult<bool> {
        // Get the current folder_id before removing
        let folder_id: Option<String> = self
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT folder_id FROM pinned_tabs WHERE id = ?1",
                    params![tab_id],
                    |row| row.get(0),
                )
                .ok()
                .flatten()
                .ok_or(rusqlite::Error::QueryReturnedNoRows)
            })
            .ok()
            .flatten();

        // Remove tab from folder
        let rows_affected = self.with_connection(|conn| {
            conn.execute(
                "UPDATE pinned_tabs SET folder_id = NULL, updated_at = datetime('now') WHERE id = ?1",
                params![tab_id],
            )
        })?;

        // If tab was in a folder, check if folder is now empty and delete if so
        if let Some(fid) = folder_id {
            self.delete_folder_if_empty(&fid)?;
        }

        Ok(rows_affected > 0)
    }

    /// Get all tabs in a folder, ordered by sort_order
    pub fn get_tabs_in_folder(&self, folder_id: &str) -> StorageResult<Vec<Tab>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, space_id, title, tab_type, content, metadata, database, is_pinned, created_at, updated_at, sort_order
                 FROM pinned_tabs WHERE folder_id = ?1 ORDER BY sort_order"
            )?;

            let tabs = stmt
                .query_map(params![folder_id], |row| {
                    let tab_type_str: String = row.get(3)?;
                    let is_pinned_int: i32 = row.get(7)?;
                    Ok(Tab {
                        id: row.get(0)?,
                        space_id: row.get(1)?,
                        title: row.get(2)?,
                        tab_type: super::tabs::TabType::from_str(&tab_type_str).unwrap_or(super::tabs::TabType::Query),
                        content: row.get(4)?,
                        metadata: row.get(5)?,
                        database: row.get(6)?,
                        folder_id: Some(folder_id.to_string()),
                        is_pinned: is_pinned_int != 0,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                        sort_order: row.get(10)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(tabs)
        })
    }

    /// Delete a folder if it has no tabs (auto-cleanup helper)
    pub fn delete_folder_if_empty(&self, folder_id: &str) -> StorageResult<bool> {
        self.with_connection(|conn| {
            // Check if folder has any tabs
            let tab_count: i32 = conn.query_row(
                "SELECT COUNT(*) FROM pinned_tabs WHERE folder_id = ?1",
                params![folder_id],
                |row| row.get(0),
            )?;

            if tab_count == 0 {
                // Folder is empty, delete it
                conn.execute("DELETE FROM tab_folders WHERE id = ?1", params![folder_id])?;
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }

    /// Reorder folders within a space
    pub fn reorder_folders(&self, space_id: &str, folder_ids: &[String]) -> StorageResult<()> {
        self.with_connection(|conn| {
            for (index, id) in folder_ids.iter().enumerate() {
                conn.execute(
                    "UPDATE tab_folders SET sort_order = ?1, updated_at = datetime('now') WHERE id = ?2 AND space_id = ?3",
                    params![index as i32, id, space_id],
                )?;
            }
            Ok(())
        })
    }

    /// Create a folder and add tabs to it atomically
    pub fn create_folder_from_tabs(
        &self,
        space_id: &str,
        name: &str,
        tab_ids: &[String],
    ) -> StorageResult<TabFolder> {
        // Create the folder first
        let folder = self.create_folder(CreateFolderInput {
            space_id: space_id.to_string(),
            name: name.to_string(),
        })?;

        // Add all tabs to the folder
        for tab_id in tab_ids {
            self.add_tab_to_folder(tab_id, &folder.id)?;
        }

        Ok(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::spaces::CreateSpaceInput;
    use crate::storage::tabs::CreateTabInput;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn create_test_db() -> (DatabaseManager, PathBuf) {
        let temp_dir = std::env::temp_dir();
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let db_path = temp_dir.join(format!(
            "larik_folders_test_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&db_path);
        let manager = DatabaseManager::new(db_path.clone()).unwrap();
        (manager, db_path)
    }

    fn create_test_space(manager: &DatabaseManager) -> String {
        manager
            .create_space(CreateSpaceInput {
                name: "Test Space".to_string(),
                color: None,
                icon: None,
                connection_host: None,
                connection_port: None,
                connection_database: None,
                connection_username: None,
                connection_password: None,
                connection_trust_cert: None,
                connection_encrypt: None,
            })
            .unwrap()
            .id
    }

    #[test]
    fn test_create_folder() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let folder = manager
            .create_folder(CreateFolderInput {
                space_id: space_id.clone(),
                name: "My Queries".to_string(),
            })
            .unwrap();

        assert_eq!(folder.name, "My Queries");
        assert_eq!(folder.space_id, space_id);
        assert!(folder.is_expanded);
        assert!(!folder.id.is_empty());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_get_folder() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let created = manager
            .create_folder(CreateFolderInput {
                space_id,
                name: "Test Folder".to_string(),
            })
            .unwrap();

        let fetched = manager.get_folder(&created.id).unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "Test Folder");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_update_folder() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let folder = manager
            .create_folder(CreateFolderInput {
                space_id,
                name: "Original".to_string(),
            })
            .unwrap();

        let updated = manager
            .update_folder(
                &folder.id,
                UpdateFolderInput {
                    name: Some("Renamed".to_string()),
                    is_expanded: Some(false),
                    sort_order: None,
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(updated.name, "Renamed");
        assert!(!updated.is_expanded);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_delete_folder() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let folder = manager
            .create_folder(CreateFolderInput {
                space_id,
                name: "To Delete".to_string(),
            })
            .unwrap();

        let deleted = manager.delete_folder(&folder.id).unwrap();
        assert!(deleted);

        let fetched = manager.get_folder(&folder.id).unwrap();
        assert!(fetched.is_none());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_add_tab_to_folder() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let folder = manager
            .create_folder(CreateFolderInput {
                space_id: space_id.clone(),
                name: "My Folder".to_string(),
            })
            .unwrap();

        let tab = manager
            .create_tab(CreateTabInput {
                space_id: space_id.clone(),
                title: "Query 1".to_string(),
                tab_type: crate::storage::tabs::TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        let added = manager.add_tab_to_folder(&tab.id, &folder.id).unwrap();
        assert!(added);

        // Verify tab is now pinned and in folder
        let updated_tab = manager.get_tab(&tab.id).unwrap().unwrap();
        assert!(updated_tab.is_pinned);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_delete_folder_if_empty() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let folder = manager
            .create_folder(CreateFolderInput {
                space_id: space_id.clone(),
                name: "Empty Folder".to_string(),
            })
            .unwrap();

        // Delete empty folder
        let deleted = manager.delete_folder_if_empty(&folder.id).unwrap();
        assert!(deleted);

        let fetched = manager.get_folder(&folder.id).unwrap();
        assert!(fetched.is_none());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_create_folder_from_tabs() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let tab1 = manager
            .create_tab(CreateTabInput {
                space_id: space_id.clone(),
                title: "Query 1".to_string(),
                tab_type: crate::storage::tabs::TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        let tab2 = manager
            .create_tab(CreateTabInput {
                space_id: space_id.clone(),
                title: "Query 2".to_string(),
                tab_type: crate::storage::tabs::TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        let folder = manager
            .create_folder_from_tabs(&space_id, "Combined", &[tab1.id.clone(), tab2.id.clone()])
            .unwrap();

        let tabs_in_folder = manager.get_tabs_in_folder(&folder.id).unwrap();
        assert_eq!(tabs_in_folder.len(), 2);

        let _ = std::fs::remove_file(&db_path);
    }
}
