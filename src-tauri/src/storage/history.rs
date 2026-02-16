// Archive/history management for closed and inactive tabs
// Implements FTS5 full-text search and auto-archive functionality

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::{DatabaseManager, StorageResult};
use super::tabs::{Tab, TabType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedTab {
    pub id: String,
    pub original_tab_id: String,
    pub space_id: Option<String>,
    pub space_name: String,
    pub title: String,
    pub tab_type: String,
    pub content: Option<String>,
    pub metadata: Option<String>,
    pub database: Option<String>,
    pub was_pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_accessed_at: String,
    pub archived_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveSearchResult {
    pub archived_tab: ArchivedTab,
    pub rank: f64,
    pub snippet_title: Option<String>,
    pub snippet_content: Option<String>,
}

impl DatabaseManager {
    /// Archive a tab (move from pinned_tabs to archived_tabs)
    pub fn archive_tab(&self, tab_id: &str) -> StorageResult<ArchivedTab> {
        self.with_connection_mut(|conn| {
            // Start transaction
            let tx = conn.transaction()?;

            // Fetch the tab
            let (tab, tab_type_str): (Tab, String) = tx.query_row(
                "SELECT id, space_id, title, tab_type, content, metadata, database, is_pinned,
                        created_at, updated_at, sort_order
                 FROM pinned_tabs WHERE id = ?",
                params![tab_id],
                |row| {
                    let tab_type_str: String = row.get(3)?;
                    let tab_type = TabType::from_str(&tab_type_str).unwrap_or(TabType::Query);
                    Ok((Tab {
                        id: row.get(0)?,
                        space_id: row.get(1)?,
                        title: row.get(2)?,
                        tab_type,
                        content: row.get(4)?,
                        metadata: row.get(5)?,
                        database: row.get(6)?,
                        folder_id: None, // Archived tabs are not in folders
                        is_pinned: row.get::<_, i32>(7)? != 0,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                        sort_order: row.get(10)?,
                    }, tab_type_str))
                },
            )?;

            // Get last_accessed_at (use created_at if NULL)
            let last_accessed_at: String = tx.query_row(
                "SELECT COALESCE(last_accessed_at, created_at) FROM pinned_tabs WHERE id = ?",
                params![tab_id],
                |row| row.get(0),
            )?;

            // Get space name
            let space_name: String = tx
                .query_row(
                    "SELECT name FROM spaces WHERE id = ?",
                    params![&tab.space_id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "Unknown Space".to_string());

            // Create archived tab
            let archived_id = Uuid::new_v4().to_string();
            let archived_tab = ArchivedTab {
                id: archived_id.clone(),
                original_tab_id: tab.id.clone(),
                space_id: Some(tab.space_id.clone()),
                space_name,
                title: tab.title.clone(),
                tab_type: tab_type_str.clone(),
                content: tab.content.clone(),
                metadata: tab.metadata.clone(),
                database: tab.database.clone(),
                was_pinned: tab.is_pinned,
                created_at: tab.created_at.clone(),
                updated_at: tab.updated_at.clone(),
                last_accessed_at,
                archived_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            };

            // Insert into archived_tabs
            tx.execute(
                "INSERT INTO archived_tabs
                 (id, original_tab_id, space_id, space_name, title, tab_type, content, metadata,
                  database, was_pinned, created_at, updated_at, last_accessed_at, archived_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    &archived_tab.id,
                    &archived_tab.original_tab_id,
                    &archived_tab.space_id,
                    &archived_tab.space_name,
                    &archived_tab.title,
                    &archived_tab.tab_type,
                    &archived_tab.content,
                    &archived_tab.metadata,
                    &archived_tab.database,
                    archived_tab.was_pinned as i32,
                    &archived_tab.created_at,
                    &archived_tab.updated_at,
                    &archived_tab.last_accessed_at,
                    &archived_tab.archived_at,
                ],
            )?;

            // Delete from pinned_tabs
            tx.execute("DELETE FROM pinned_tabs WHERE id = ?", params![tab_id])?;

            tx.commit()?;

            Ok(archived_tab)
        })
    }

    /// Restore an archived tab back to active tabs
    pub fn restore_archived_tab(
        &self,
        archived_id: &str,
        target_space_id: &str,
    ) -> StorageResult<Tab> {
        self.with_connection_mut(|conn| {
            let tx = conn.transaction()?;

            // Fetch archived tab
            let archived: ArchivedTab = tx.query_row(
                "SELECT id, original_tab_id, space_id, space_name, title, tab_type, content,
                        metadata, database, was_pinned, created_at, updated_at, last_accessed_at, archived_at
                 FROM archived_tabs WHERE id = ?",
                params![archived_id],
                |row| {
                    Ok(ArchivedTab {
                        id: row.get(0)?,
                        original_tab_id: row.get(1)?,
                        space_id: row.get(2)?,
                        space_name: row.get(3)?,
                        title: row.get(4)?,
                        tab_type: row.get(5)?,
                        content: row.get(6)?,
                        metadata: row.get(7)?,
                        database: row.get(8)?,
                        was_pinned: row.get::<_, i32>(9)? != 0,
                        created_at: row.get(10)?,
                        updated_at: row.get(11)?,
                        last_accessed_at: row.get(12)?,
                        archived_at: row.get(13)?,
                    })
                },
            )?;

            // Get max sort_order for target space
            let max_sort_order: i32 = tx
                .query_row(
                    "SELECT COALESCE(MAX(sort_order), 0) FROM pinned_tabs WHERE space_id = ?",
                    params![target_space_id],
                    |row| row.get(0),
                )?;

            // Create new tab in target space
            let new_tab_id = Uuid::new_v4().to_string();
            let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let tab_type = TabType::from_str(&archived.tab_type).unwrap_or(TabType::Query);

            tx.execute(
                "INSERT INTO pinned_tabs
                 (id, space_id, title, tab_type, content, metadata, database, is_pinned,
                  created_at, updated_at, sort_order, last_accessed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    &new_tab_id,
                    target_space_id,
                    &archived.title,
                    &archived.tab_type,
                    &archived.content,
                    &archived.metadata,
                    &archived.database,
                    archived.was_pinned as i32,
                    &archived.created_at,
                    &now,
                    max_sort_order + 1,
                    &now,
                ],
            )?;

            // Delete from archived_tabs
            tx.execute("DELETE FROM archived_tabs WHERE id = ?", params![archived_id])?;

            tx.commit()?;

            // Return the new tab
            Ok(Tab {
                id: new_tab_id,
                space_id: target_space_id.to_string(),
                title: archived.title,
                tab_type,
                content: archived.content,
                metadata: archived.metadata,
                database: archived.database,
                folder_id: None, // Restored tabs are not in folders
                is_pinned: archived.was_pinned,
                created_at: archived.created_at,
                updated_at: now,
                sort_order: max_sort_order + 1,
            })
        })
    }

    /// Search archived tabs using FTS5 full-text search with LIKE fallback
    pub fn search_archived_tabs(
        &self,
        query: &str,
        space_id: Option<&str>,
        limit: Option<usize>,
    ) -> StorageResult<Vec<ArchiveSearchResult>> {
        let limit = limit.unwrap_or(50);
        let query_trimmed = query.trim();

        // Return empty results if query is empty
        if query_trimmed.is_empty() {
            return Ok(Vec::new());
        }

        // Try FTS5 search first, fall back to LIKE-based search on any error
        match self.search_archived_tabs_fts(query_trimmed, space_id, limit) {
            Ok(results) => Ok(results),
            Err(_) => {
                // FTS5 failed (possibly corrupted index), use LIKE-based fallback
                self.search_archived_tabs_like(query_trimmed, space_id, limit)
            }
        }
    }

    /// FTS5-based search (may fail if index is corrupted)
    fn search_archived_tabs_fts(
        &self,
        query: &str,
        space_id: Option<&str>,
        limit: usize,
    ) -> StorageResult<Vec<ArchiveSearchResult>> {
        // Sanitize query for FTS5: remove special characters that could cause syntax errors
        let search_query = query
            .split_whitespace()
            .filter_map(|word| {
                // Remove FTS5 special characters, keep only alphanumeric and underscore
                let sanitized: String = word
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                
                if sanitized.is_empty() {
                    return None;
                }
                
                // FTS5 reserved keywords need to be quoted
                let upper = sanitized.to_uppercase();
                if upper == "AND" || upper == "OR" || upper == "NOT" || upper == "NEAR" {
                    return Some(format!("\"{}\"", sanitized));
                }
                
                // Append * for prefix matching
                Some(format!("{}*", sanitized))
            })
            .collect::<Vec<_>>()
            .join(" ");

        if search_query.is_empty() {
            return Ok(Vec::new());
        }

        self.with_connection(|conn| {
            // Build query with optional space_id filter
            let (sql, params): (String, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(sid) = space_id {
                (
                    "SELECT
                        at.id, at.original_tab_id, at.space_id, at.space_name, at.title, at.tab_type,
                        at.content, at.metadata, at.database, at.was_pinned, at.created_at,
                        at.updated_at, at.last_accessed_at, at.archived_at,
                        fts.rank,
                        snippet(archived_tabs_fts, 0, '<mark>', '</mark>', '...', 32) as snippet_title,
                        snippet(archived_tabs_fts, 1, '<mark>', '</mark>', '...', 64) as snippet_content
                     FROM archived_tabs_fts fts
                     JOIN archived_tabs at ON fts.rowid = at.rowid
                     WHERE archived_tabs_fts MATCH ? AND at.space_id = ?
                     ORDER BY rank
                     LIMIT ?".to_string(),
                    vec![Box::new(search_query.clone()), Box::new(sid.to_string()), Box::new(limit as i64)],
                )
            } else {
                (
                    "SELECT
                        at.id, at.original_tab_id, at.space_id, at.space_name, at.title, at.tab_type,
                        at.content, at.metadata, at.database, at.was_pinned, at.created_at,
                        at.updated_at, at.last_accessed_at, at.archived_at,
                        fts.rank,
                        snippet(archived_tabs_fts, 0, '<mark>', '</mark>', '...', 32) as snippet_title,
                        snippet(archived_tabs_fts, 1, '<mark>', '</mark>', '...', 64) as snippet_content
                     FROM archived_tabs_fts fts
                     JOIN archived_tabs at ON fts.rowid = at.rowid
                     WHERE archived_tabs_fts MATCH ?
                     ORDER BY rank
                     LIMIT ?".to_string(),
                    vec![Box::new(search_query.clone()), Box::new(limit as i64)],
                )
            };

            let mut stmt = conn.prepare(&sql)?;
            let results = stmt
                .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                    Ok(ArchiveSearchResult {
                        archived_tab: ArchivedTab {
                            id: row.get(0)?,
                            original_tab_id: row.get(1)?,
                            space_id: row.get(2)?,
                            space_name: row.get(3)?,
                            title: row.get(4)?,
                            tab_type: row.get(5)?,
                            content: row.get(6)?,
                            metadata: row.get(7)?,
                            database: row.get(8)?,
                            was_pinned: row.get::<_, i32>(9)? != 0,
                            created_at: row.get(10)?,
                            updated_at: row.get(11)?,
                            last_accessed_at: row.get(12)?,
                            archived_at: row.get(13)?,
                        },
                        rank: row.get(14)?,
                        snippet_title: row.get(15)?,
                        snippet_content: row.get(16)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(results)
        })
    }

    /// LIKE-based search fallback when FTS5 fails
    fn search_archived_tabs_like(
        &self,
        query: &str,
        space_id: Option<&str>,
        limit: usize,
    ) -> StorageResult<Vec<ArchiveSearchResult>> {
        // Create LIKE pattern for each word
        let like_pattern = format!("%{}%", query.replace('%', "").replace('_', ""));

        self.with_connection(|conn| {
            // Build query with optional space_id filter
            let (sql, params): (String, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(sid) = space_id {
                (
                    "SELECT
                        id, original_tab_id, space_id, space_name, title, tab_type,
                        content, metadata, database, was_pinned, created_at,
                        updated_at, last_accessed_at, archived_at
                     FROM archived_tabs
                     WHERE space_id = ? AND (title LIKE ? COLLATE NOCASE OR content LIKE ? COLLATE NOCASE)
                     ORDER BY archived_at DESC
                     LIMIT ?".to_string(),
                    vec![Box::new(sid.to_string()), Box::new(like_pattern.clone()), Box::new(like_pattern.clone()), Box::new(limit as i64)],
                )
            } else {
                (
                    "SELECT
                        id, original_tab_id, space_id, space_name, title, tab_type,
                        content, metadata, database, was_pinned, created_at,
                        updated_at, last_accessed_at, archived_at
                     FROM archived_tabs
                     WHERE title LIKE ? COLLATE NOCASE OR content LIKE ? COLLATE NOCASE
                     ORDER BY archived_at DESC
                     LIMIT ?".to_string(),
                    vec![Box::new(like_pattern.clone()), Box::new(like_pattern.clone()), Box::new(limit as i64)],
                )
            };

            let mut stmt = conn.prepare(&sql)?;
            let results = stmt
                .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                    let title: String = row.get(4)?;
                    let content: Option<String> = row.get(6)?;
                    
                    Ok(ArchiveSearchResult {
                        archived_tab: ArchivedTab {
                            id: row.get(0)?,
                            original_tab_id: row.get(1)?,
                            space_id: row.get(2)?,
                            space_name: row.get(3)?,
                            title: title.clone(),
                            tab_type: row.get(5)?,
                            content: content.clone(),
                            metadata: row.get(7)?,
                            database: row.get(8)?,
                            was_pinned: row.get::<_, i32>(9)? != 0,
                            created_at: row.get(10)?,
                            updated_at: row.get(11)?,
                            last_accessed_at: row.get(12)?,
                            archived_at: row.get(13)?,
                        },
                        rank: 0.0, // No ranking for LIKE search
                        snippet_title: Some(title),
                        snippet_content: content,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(results)
        })
    }

    /// Get archived tabs with optional space filter and pagination
    pub fn get_archived_tabs(
        &self,
        space_id: Option<&str>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> StorageResult<Vec<ArchivedTab>> {
        let limit = limit.unwrap_or(100);
        let offset = offset.unwrap_or(0);

        self.with_connection(|conn| {
            let (query, params): (String, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(sid) = space_id {
                (
                    "SELECT id, original_tab_id, space_id, space_name, title, tab_type, content,
                            metadata, database, was_pinned, created_at, updated_at, last_accessed_at, archived_at
                     FROM archived_tabs
                     WHERE space_id = ?
                     ORDER BY archived_at DESC
                     LIMIT ? OFFSET ?".to_string(),
                    vec![Box::new(sid.to_string()), Box::new(limit as i64), Box::new(offset as i64)],
                )
            } else {
                (
                    "SELECT id, original_tab_id, space_id, space_name, title, tab_type, content,
                            metadata, database, was_pinned, created_at, updated_at, last_accessed_at, archived_at
                     FROM archived_tabs
                     ORDER BY archived_at DESC
                     LIMIT ? OFFSET ?".to_string(),
                    vec![Box::new(limit as i64), Box::new(offset as i64)],
                )
            };

            let mut stmt = conn.prepare(&query)?;
            let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

            let results = stmt
                .query_map(params_refs.as_slice(), |row| {
                    Ok(ArchivedTab {
                        id: row.get(0)?,
                        original_tab_id: row.get(1)?,
                        space_id: row.get(2)?,
                        space_name: row.get(3)?,
                        title: row.get(4)?,
                        tab_type: row.get(5)?,
                        content: row.get(6)?,
                        metadata: row.get(7)?,
                        database: row.get(8)?,
                        was_pinned: row.get::<_, i32>(9)? != 0,
                        created_at: row.get(10)?,
                        updated_at: row.get(11)?,
                        last_accessed_at: row.get(12)?,
                        archived_at: row.get(13)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(results)
        })
    }

    /// Get count of archived tabs with optional space filter
    pub fn get_archived_tabs_count(&self, space_id: Option<&str>) -> StorageResult<usize> {
        self.with_connection(|conn| {
            let count: i64 = if let Some(sid) = space_id {
                conn.query_row(
                    "SELECT COUNT(*) FROM archived_tabs WHERE space_id = ?",
                    params![sid],
                    |row| row.get(0),
                )?
            } else {
                conn.query_row("SELECT COUNT(*) FROM archived_tabs", [], |row| row.get(0))?
            };

            Ok(count as usize)
        })
    }

    /// Permanently delete an archived tab
    pub fn delete_archived_tab(&self, archived_id: &str) -> StorageResult<bool> {
        self.with_connection(|conn| {
            let affected = conn.execute("DELETE FROM archived_tabs WHERE id = ?", params![archived_id])?;
            Ok(affected > 0)
        })
    }

    /// Cleanup archived tabs older than specified retention period
    pub fn cleanup_old_archived_tabs(&self, retention_days: i32) -> StorageResult<usize> {
        self.with_connection(|conn| {
            let affected = conn.execute(
                "DELETE FROM archived_tabs
                 WHERE julianday('now') - julianday(archived_at) > ?",
                params![retention_days],
            )?;
            Ok(affected)
        })
    }

    /// Find tabs inactive for specified number of days (excludes pinned tabs)
    pub fn find_inactive_tabs(&self, days_inactive: i32) -> StorageResult<Vec<Tab>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, space_id, title, tab_type, content, metadata, database, folder_id, is_pinned,
                        created_at, updated_at, sort_order
                 FROM pinned_tabs
                 WHERE is_pinned = 0
                   AND julianday('now') - julianday(last_accessed_at) > ?
                 ORDER BY last_accessed_at ASC",
            )?;

            let tabs = stmt
                .query_map(params![days_inactive], |row| {
                    let tab_type_str: String = row.get(3)?;
                    let tab_type = TabType::from_str(&tab_type_str).unwrap_or(TabType::Query);
                    Ok(Tab {
                        id: row.get(0)?,
                        space_id: row.get(1)?,
                        title: row.get(2)?,
                        tab_type,
                        content: row.get(4)?,
                        metadata: row.get(5)?,
                        database: row.get(6)?,
                        folder_id: row.get(7)?,
                        is_pinned: row.get::<_, i32>(8)? != 0,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                        sort_order: row.get(11)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(tabs)
        })
    }

    /// Update last_accessed_at timestamp for a tab
    pub fn touch_tab(&self, tab_id: &str) -> StorageResult<bool> {
        self.with_connection(|conn| {
            let affected = conn.execute(
                "UPDATE pinned_tabs SET last_accessed_at = datetime('now') WHERE id = ?",
                params![tab_id],
            )?;
            Ok(affected > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::DatabaseManager;
    use std::path::PathBuf;

    fn create_test_db() -> DatabaseManager {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("larik_history_test_{}.db", Uuid::new_v4()));
        DatabaseManager::new(db_path).unwrap()
    }

    #[test]
    fn test_archive_and_restore_tab() {
        let db = create_test_db();

        // Create a space and tab
        let space_id = Uuid::new_v4().to_string();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO spaces (id, name, sort_order) VALUES (?, ?, ?)",
                params![&space_id, "Test Space", 0],
            )
        })
        .unwrap();

        let tab_id = Uuid::new_v4().to_string();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO pinned_tabs (id, space_id, title, tab_type, content, sort_order, is_pinned)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![&tab_id, &space_id, "Test Tab", "query", "SELECT * FROM test", 0, 0],
            )
        })
        .unwrap();

        // Archive the tab
        let archived = db.archive_tab(&tab_id).unwrap();
        assert_eq!(archived.title, "Test Tab");
        assert_eq!(archived.space_name, "Test Space");

        // Verify tab no longer in pinned_tabs
        let count: i64 = db
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM pinned_tabs WHERE id = ?", params![&tab_id], |row| {
                    row.get(0)
                })
            })
            .unwrap();
        assert_eq!(count, 0);

        // Restore the tab
        let restored = db.restore_archived_tab(&archived.id, &space_id).unwrap();
        assert_eq!(restored.title, "Test Tab");
        assert_eq!(restored.content, Some("SELECT * FROM test".to_string()));

        // Verify archived tab is gone
        let archived_count: i64 = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM archived_tabs WHERE id = ?",
                    params![&archived.id],
                    |row| row.get(0),
                )
            })
            .unwrap();
        assert_eq!(archived_count, 0);
    }

    #[test]
    fn test_search_archived_tabs() {
        let db = create_test_db();

        // Create space
        let space_id = Uuid::new_v4().to_string();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO spaces (id, name, sort_order) VALUES (?, ?, ?)",
                params![&space_id, "Test Space", 0],
            )
        })
        .unwrap();

        // Create and archive multiple tabs
        for i in 1..=3 {
            let tab_id = Uuid::new_v4().to_string();
            db.with_connection(|conn| {
                conn.execute(
                    "INSERT INTO pinned_tabs (id, space_id, title, tab_type, content, sort_order, is_pinned)
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![
                        &tab_id,
                        &space_id,
                        format!("Query {}", i),
                        "query",
                        format!("SELECT * FROM users WHERE id = {}", i),
                        i,
                        0
                    ],
                )
            })
            .unwrap();
            db.archive_tab(&tab_id).unwrap();
        }

        // Search by title
        let results = db.search_archived_tabs("Query", None, Some(10)).unwrap();
        assert_eq!(results.len(), 3);

        // Search by content
        let results = db.search_archived_tabs("users", None, Some(10)).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_find_inactive_tabs() {
        let db = create_test_db();

        // Create space
        let space_id = Uuid::new_v4().to_string();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO spaces (id, name, sort_order) VALUES (?, ?, ?)",
                params![&space_id, "Test Space", 0],
            )
        })
        .unwrap();

        // Create tabs with old last_accessed_at
        let tab_id = Uuid::new_v4().to_string();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO pinned_tabs (id, space_id, title, tab_type, sort_order, is_pinned, last_accessed_at)
                 VALUES (?, ?, ?, ?, ?, ?, datetime('now', '-15 days'))",
                params![&tab_id, &space_id, "Old Tab", "query", 0, 0],
            )
        })
        .unwrap();

        // Find inactive tabs
        let inactive = db.find_inactive_tabs(14).unwrap();
        assert_eq!(inactive.len(), 1);
        assert_eq!(inactive[0].title, "Old Tab");
    }

    #[test]
    fn test_cleanup_old_archived_tabs() {
        let db = create_test_db();

        // Create space
        let space_id = Uuid::new_v4().to_string();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO spaces (id, name, sort_order) VALUES (?, ?, ?)",
                params![&space_id, "Test Space", 0],
            )
        })
        .unwrap();

        // Create and archive a tab with old archived_at
        let tab_id = Uuid::new_v4().to_string();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO pinned_tabs (id, space_id, title, tab_type, sort_order, is_pinned)
                 VALUES (?, ?, ?, ?, ?, ?)",
                params![&tab_id, &space_id, "Old Tab", "query", 0, 0],
            )
        })
        .unwrap();

        let archived = db.archive_tab(&tab_id).unwrap();

        // Manually update archived_at to be old
        db.with_connection(|conn| {
            conn.execute(
                "UPDATE archived_tabs SET archived_at = datetime('now', '-91 days') WHERE id = ?",
                params![&archived.id],
            )
        })
        .unwrap();

        // Cleanup
        let deleted = db.cleanup_old_archived_tabs(90).unwrap();
        assert_eq!(deleted, 1);
    }
}
