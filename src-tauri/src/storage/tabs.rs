// Tabs data model and storage operations
// Tabs represent query editors, pinned to spaces

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::{DatabaseManager, StorageResult};

/// Tab type enum for different kinds of tabs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TabType {
    Query,
    Results,
    Schema,
    Settings,
}

impl TabType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TabType::Query => "query",
            TabType::Results => "results",
            TabType::Schema => "schema",
            TabType::Settings => "settings",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "query" => Some(TabType::Query),
            "results" => Some(TabType::Results),
            "schema" => Some(TabType::Schema),
            "settings" => Some(TabType::Settings),
            _ => None,
        }
    }
}

/// A Tab represents a tab within a space (can be pinned or unpinned)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tab {
    pub id: String,
    pub space_id: String,
    pub title: String,
    pub tab_type: TabType,
    pub content: Option<String>,
    pub metadata: Option<String>,
    pub database: Option<String>,  // Per-tab database selection
    pub folder_id: Option<String>, // Folder this tab belongs to (Arc Browser-style)
    pub is_pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub sort_order: i32,
}

/// Input for creating a new tab
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTabInput {
    pub space_id: String,
    pub title: String,
    pub tab_type: TabType,
    pub content: Option<String>,
    pub metadata: Option<String>,
    pub database: Option<String>,
}

/// Input for updating an existing tab
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTabInput {
    pub title: Option<String>,
    pub content: Option<String>,
    pub metadata: Option<String>,
    pub database: Option<String>,
    pub sort_order: Option<i32>,
}

impl DatabaseManager {
    /// Create a new tab
    pub fn create_tab(&self, input: CreateTabInput) -> StorageResult<Tab> {
        let id = Uuid::new_v4().to_string();

        // Get the minimum sort order for this space (so new tabs appear at the top)
        let min_order: i32 = self.with_connection(|conn| {
            conn.query_row(
                "SELECT COALESCE(MIN(sort_order), 1) FROM pinned_tabs WHERE space_id = ?1",
                params![input.space_id],
                |row| row.get(0),
            )
        })?;
        let sort_order = min_order - 1;

        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO pinned_tabs (id, space_id, title, tab_type, content, metadata, database, folder_id, is_pinned, sort_order, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 0, ?8, datetime('now'), datetime('now'))
                "#,
                params![
                    id,
                    input.space_id,
                    input.title,
                    input.tab_type.as_str(),
                    input.content,
                    input.metadata,
                    input.database,
                    sort_order
                ],
            )?;
            Ok(())
        })?;

        self.get_tab(&id)?.ok_or_else(|| {
            super::database::StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows)
        })
    }

    /// Get a tab by ID
    pub fn get_tab(&self, id: &str) -> StorageResult<Option<Tab>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, space_id, title, tab_type, content, metadata, database, folder_id, is_pinned, created_at, updated_at, sort_order
                 FROM pinned_tabs WHERE id = ?1"
            )?;

            let result = stmt.query_row(params![id], |row| {
                let tab_type_str: String = row.get(3)?;
                let is_pinned_int: i32 = row.get(8)?;
                Ok(Tab {
                    id: row.get(0)?,
                    space_id: row.get(1)?,
                    title: row.get(2)?,
                    tab_type: TabType::from_str(&tab_type_str).unwrap_or(TabType::Query),
                    content: row.get(4)?,
                    metadata: row.get(5)?,
                    database: row.get(6)?,
                    folder_id: row.get(7)?,
                    is_pinned: is_pinned_int != 0,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                    sort_order: row.get(11)?,
                })
            });

            match result {
                Ok(tab) => Ok(Some(tab)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Get all tabs for a space, ordered by is_pinned (desc) then sort_order
    pub fn get_tabs_by_space(&self, space_id: &str) -> StorageResult<Vec<Tab>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, space_id, title, tab_type, content, metadata, database, folder_id, is_pinned, created_at, updated_at, sort_order
                 FROM pinned_tabs WHERE space_id = ?1 ORDER BY is_pinned DESC, sort_order"
            )?;

            let tabs = stmt
                .query_map(params![space_id], |row| {
                    let tab_type_str: String = row.get(3)?;
                    let is_pinned_int: i32 = row.get(8)?;
                    Ok(Tab {
                        id: row.get(0)?,
                        space_id: row.get(1)?,
                        title: row.get(2)?,
                        tab_type: TabType::from_str(&tab_type_str).unwrap_or(TabType::Query),
                        content: row.get(4)?,
                        metadata: row.get(5)?,
                        database: row.get(6)?,
                        folder_id: row.get(7)?,
                        is_pinned: is_pinned_int != 0,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                        sort_order: row.get(11)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(tabs)
        })
    }

    /// Update an existing tab
    pub fn update_tab(&self, id: &str, input: UpdateTabInput) -> StorageResult<Option<Tab>> {
        // Check if tab exists
        if self.get_tab(id)?.is_none() {
            return Ok(None);
        }

        self.with_connection(|conn| {
            let mut updates = vec!["updated_at = datetime('now')"];
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![];

            if let Some(ref title) = input.title {
                updates.push("title = ?");
                params_vec.push(Box::new(title.clone()));
            }
            if let Some(ref content) = input.content {
                updates.push("content = ?");
                params_vec.push(Box::new(content.clone()));
            }
            if let Some(ref metadata) = input.metadata {
                updates.push("metadata = ?");
                params_vec.push(Box::new(metadata.clone()));
            }
            if let Some(ref database) = input.database {
                updates.push("database = ?");
                params_vec.push(Box::new(database.clone()));
            }
            if let Some(sort_order) = input.sort_order {
                updates.push("sort_order = ?");
                params_vec.push(Box::new(sort_order));
            }

            params_vec.push(Box::new(id.to_string()));

            let sql = format!("UPDATE pinned_tabs SET {} WHERE id = ?", updates.join(", "));

            conn.execute(
                &sql,
                rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            )?;
            Ok(())
        })?;

        self.get_tab(id)
    }

    /// Update the database selection for a tab
    pub fn update_tab_database(&self, id: &str, database: Option<&str>) -> StorageResult<bool> {
        self.with_connection(|conn| {
            let rows_affected = conn.execute(
                "UPDATE pinned_tabs SET database = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![database, id],
            )?;
            Ok(rows_affected > 0)
        })
    }

    /// Auto-save: Update only the content of a tab (optimized for frequent saves)
    pub fn autosave_tab_content(&self, id: &str, content: &str) -> StorageResult<bool> {
        self.with_connection(|conn| {
            let rows_affected = conn.execute(
                "UPDATE pinned_tabs SET content = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![content, id],
            )?;
            Ok(rows_affected > 0)
        })
    }

    /// Toggle the pinned status of a tab
    pub fn toggle_tab_pinned(&self, id: &str) -> StorageResult<Option<Tab>> {
        self.with_connection(|conn| {
            let rows_affected = conn.execute(
                "UPDATE pinned_tabs SET is_pinned = NOT is_pinned, updated_at = datetime('now') WHERE id = ?1",
                params![id],
            )?;
            if rows_affected == 0 {
                return Ok(());
            }
            Ok(())
        })?;
        self.get_tab(id)
    }

    /// Delete a tab by ID
    pub fn delete_tab(&self, id: &str) -> StorageResult<bool> {
        // Get the folder_id before deleting (for auto-cleanup)
        let folder_id: Option<String> = self
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT folder_id FROM pinned_tabs WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
            })
            .ok();

        // Delete the tab
        let deleted = self.with_connection(|conn| {
            let rows_affected =
                conn.execute("DELETE FROM pinned_tabs WHERE id = ?1", params![id])?;
            Ok(rows_affected > 0)
        })?;

        // If tab was in a folder, check if folder is now empty and delete if so
        if deleted {
            if let Some(fid) = folder_id {
                self.delete_folder_if_empty(&fid)?;
            }
        }

        Ok(deleted)
    }

    /// Reorder tabs within a space
    pub fn reorder_tabs(&self, space_id: &str, tab_ids: &[String]) -> StorageResult<()> {
        self.with_connection(|conn| {
            for (index, id) in tab_ids.iter().enumerate() {
                conn.execute(
                    "UPDATE pinned_tabs SET sort_order = ?1, updated_at = datetime('now') WHERE id = ?2 AND space_id = ?3",
                    params![index as i32, id, space_id],
                )?;
            }
            Ok(())
        })
    }

    /// Move a tab to a different space
    pub fn move_tab_to_space(
        &self,
        tab_id: &str,
        new_space_id: &str,
    ) -> StorageResult<Option<Tab>> {
        // Get min sort order in the new space (so moved tabs appear at the top)
        let min_order: i32 = self.with_connection(|conn| {
            conn.query_row(
                "SELECT COALESCE(MIN(sort_order), 1) FROM pinned_tabs WHERE space_id = ?1",
                params![new_space_id],
                |row| row.get(0),
            )
        })?;

        self.with_connection(|conn| {
            let rows_affected = conn.execute(
                "UPDATE pinned_tabs SET space_id = ?1, sort_order = ?2, updated_at = datetime('now') WHERE id = ?3",
                params![new_space_id, min_order - 1, tab_id],
            )?;
            if rows_affected == 0 {
                return Ok(());
            }
            Ok(())
        })?;

        self.get_tab(tab_id)
    }

    /// Search active tabs by title or content
    pub fn search_tabs(&self, query: &str) -> StorageResult<Vec<Tab>> {
        let query_pattern = format!("%{}%", query);

        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, space_id, title, tab_type, content, metadata, database, folder_id, is_pinned, created_at, updated_at, sort_order
                 FROM pinned_tabs 
                 WHERE title LIKE ?1 OR content LIKE ?2
                 ORDER BY updated_at DESC
                 LIMIT 50"
            )?;

            let tabs = stmt
                .query_map(params![query_pattern, query_pattern], |row| {
                    let tab_type_str: String = row.get(3)?;
                    let is_pinned_int: i32 = row.get(8)?;
                    Ok(Tab {
                        id: row.get(0)?,
                        space_id: row.get(1)?,
                        title: row.get(2)?,
                        tab_type: TabType::from_str(&tab_type_str).unwrap_or(TabType::Query),
                        content: row.get(4)?,
                        metadata: row.get(5)?,
                        database: row.get(6)?,
                        folder_id: row.get(7)?,
                        is_pinned: is_pinned_int != 0,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                        sort_order: row.get(11)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(tabs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::spaces::CreateSpaceInput;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn create_test_db() -> (DatabaseManager, PathBuf) {
        let temp_dir = std::env::temp_dir();
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let db_path = temp_dir.join(format!(
            "larik_tabs_test_{}_{}.db",
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
    fn test_create_tab() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let tab = manager
            .create_tab(CreateTabInput {
                space_id: space_id.clone(),
                title: "My Query".to_string(),
                tab_type: TabType::Query,
                content: Some("SELECT * FROM users".to_string()),
                metadata: None,
                database: None,
            })
            .unwrap();

        assert_eq!(tab.title, "My Query");
        assert_eq!(tab.tab_type, TabType::Query);
        assert_eq!(tab.content, Some("SELECT * FROM users".to_string()));
        assert_eq!(tab.sort_order, 0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_get_tab() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let created = manager
            .create_tab(CreateTabInput {
                space_id,
                title: "Test Tab".to_string(),
                tab_type: TabType::Schema,
                content: None,
                metadata: Some(r#"{"table": "users"}"#.to_string()),
                database: None,
            })
            .unwrap();

        let fetched = manager.get_tab(&created.id).unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.tab_type, TabType::Schema);
        assert_eq!(fetched.metadata, Some(r#"{"table": "users"}"#.to_string()));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_get_tabs_by_space() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        manager
            .create_tab(CreateTabInput {
                space_id: space_id.clone(),
                title: "Tab A".to_string(),
                tab_type: TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        manager
            .create_tab(CreateTabInput {
                space_id: space_id.clone(),
                title: "Tab B".to_string(),
                tab_type: TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        let tabs = manager.get_tabs_by_space(&space_id).unwrap();
        assert_eq!(tabs.len(), 2);
        // Newest tab (B) should be first
        assert_eq!(tabs[0].title, "Tab B");
        assert_eq!(tabs[1].title, "Tab A");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_update_tab() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let tab = manager
            .create_tab(CreateTabInput {
                space_id,
                title: "Original".to_string(),
                tab_type: TabType::Query,
                content: Some("SELECT 1".to_string()),
                metadata: None,
                database: None,
            })
            .unwrap();

        let updated = manager
            .update_tab(
                &tab.id,
                UpdateTabInput {
                    title: Some("Renamed".to_string()),
                    content: Some("SELECT 2".to_string()),
                    metadata: None,
                    database: None,
                    sort_order: None,
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(updated.title, "Renamed");
        assert_eq!(updated.content, Some("SELECT 2".to_string()));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_autosave_tab_content() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let tab = manager
            .create_tab(CreateTabInput {
                space_id,
                title: "Autosave Test".to_string(),
                tab_type: TabType::Query,
                content: Some("Initial content".to_string()),
                metadata: None,
                database: None,
            })
            .unwrap();

        let saved = manager
            .autosave_tab_content(&tab.id, "Updated content")
            .unwrap();
        assert!(saved);

        let fetched = manager.get_tab(&tab.id).unwrap().unwrap();
        assert_eq!(fetched.content, Some("Updated content".to_string()));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_delete_tab() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let tab = manager
            .create_tab(CreateTabInput {
                space_id,
                title: "To Delete".to_string(),
                tab_type: TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        let deleted = manager.delete_tab(&tab.id).unwrap();
        assert!(deleted);

        let fetched = manager.get_tab(&tab.id).unwrap();
        assert!(fetched.is_none());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_reorder_tabs() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let tab_a = manager
            .create_tab(CreateTabInput {
                space_id: space_id.clone(),
                title: "A".to_string(),
                tab_type: TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        let tab_b = manager
            .create_tab(CreateTabInput {
                space_id: space_id.clone(),
                title: "B".to_string(),
                tab_type: TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        // Reorder: B, A
        manager
            .reorder_tabs(&space_id, &[tab_b.id.clone(), tab_a.id.clone()])
            .unwrap();

        let tabs = manager.get_tabs_by_space(&space_id).unwrap();
        assert_eq!(tabs[0].title, "B");
        assert_eq!(tabs[1].title, "A");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_move_tab_to_space() {
        let (manager, db_path) = create_test_db();

        let space1_id = create_test_space(&manager);
        let space2 = manager
            .create_space(CreateSpaceInput {
                name: "Space 2".to_string(),
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
            .unwrap();

        let tab = manager
            .create_tab(CreateTabInput {
                space_id: space1_id.clone(),
                title: "Moving Tab".to_string(),
                tab_type: TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        manager.move_tab_to_space(&tab.id, &space2.id).unwrap();

        let moved = manager.get_tab(&tab.id).unwrap().unwrap();
        assert_eq!(moved.space_id, space2.id);

        let space1_tabs = manager.get_tabs_by_space(&space1_id).unwrap();
        assert_eq!(space1_tabs.len(), 0);

        let space2_tabs = manager.get_tabs_by_space(&space2.id).unwrap();
        assert_eq!(space2_tabs.len(), 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_cascade_delete_tabs_on_space_delete() {
        let (manager, db_path) = create_test_db();
        let space_id = create_test_space(&manager);

        let tab = manager
            .create_tab(CreateTabInput {
                space_id: space_id.clone(),
                title: "Will be deleted".to_string(),
                tab_type: TabType::Query,
                content: None,
                metadata: None,
                database: None,
            })
            .unwrap();

        // Delete the space
        manager.delete_space(&space_id).unwrap();

        // Tab should be cascade deleted
        let fetched = manager.get_tab(&tab.id).unwrap();
        assert!(fetched.is_none());

        let _ = std::fs::remove_file(&db_path);
    }
}
