// Database connection manager for local SQLite storage
// Handles app data directory resolution and connection pooling

use rusqlite::{Connection, Result as SqliteResult};
use std::path::PathBuf;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Failed to get app data directory")]
    AppDataDir,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Database manager for local SQLite storage
pub struct DatabaseManager {
    connection: Mutex<Connection>,
    db_path: PathBuf,
}

impl DatabaseManager {
    /// Create a new database manager with the given database path
    pub fn new(db_path: PathBuf) -> StorageResult<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(&db_path)?;
        
        // Enable foreign keys
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;

        let manager = Self {
            connection: Mutex::new(connection),
            db_path,
        };

        // Initialize schema
        manager.init_schema()?;

        Ok(manager)
    }

    /// Get the database path
    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    /// Initialize the database schema
    fn init_schema(&self) -> StorageResult<()> {
        let conn = self.connection.lock().unwrap();
        
        conn.execute_batch(
            r#"
            -- Spaces table: work environments/workspaces with 1:1 database connection
            CREATE TABLE IF NOT EXISTS spaces (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                color TEXT,
                icon TEXT,
                -- Connection fields (1 connection per space)
                connection_host TEXT,
                connection_port INTEGER DEFAULT 1433,
                connection_database TEXT,
                connection_username TEXT,
                connection_password TEXT,
                connection_trust_cert INTEGER DEFAULT 1,
                connection_encrypt INTEGER DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                sort_order INTEGER NOT NULL DEFAULT 0
            );

            -- Pinned tabs table: tabs within spaces (can be pinned or unpinned)
            CREATE TABLE IF NOT EXISTS pinned_tabs (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                title TEXT NOT NULL,
                tab_type TEXT NOT NULL,
                content TEXT,
                metadata TEXT,
                database TEXT,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                sort_order INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (space_id) REFERENCES spaces(id) ON DELETE CASCADE
            );

            -- App state table: general UI state persistence
            CREATE TABLE IF NOT EXISTS app_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Create indexes for common queries
            CREATE INDEX IF NOT EXISTS idx_pinned_tabs_space_id ON pinned_tabs(space_id);
            CREATE INDEX IF NOT EXISTS idx_spaces_sort_order ON spaces(sort_order);
            CREATE INDEX IF NOT EXISTS idx_pinned_tabs_sort_order ON pinned_tabs(sort_order);
            "#,
        )?;

        // Migration: Add is_pinned column if it doesn't exist (for existing databases)
        let has_is_pinned: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('pinned_tabs') WHERE name = 'is_pinned'",
            [],
            |row| row.get(0),
        )?;
        
        if !has_is_pinned {
            conn.execute(
                "ALTER TABLE pinned_tabs ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }

        // Migration: Add connection fields to spaces if they don't exist
        let has_connection_host: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('spaces') WHERE name = 'connection_host'",
            [],
            |row| row.get(0),
        )?;
        
        if !has_connection_host {
            conn.execute_batch(
                r#"
                ALTER TABLE spaces ADD COLUMN connection_host TEXT;
                ALTER TABLE spaces ADD COLUMN connection_port INTEGER DEFAULT 1433;
                ALTER TABLE spaces ADD COLUMN connection_database TEXT;
                ALTER TABLE spaces ADD COLUMN connection_username TEXT;
                ALTER TABLE spaces ADD COLUMN connection_password TEXT;
                ALTER TABLE spaces ADD COLUMN connection_trust_cert INTEGER DEFAULT 1;
                ALTER TABLE spaces ADD COLUMN connection_encrypt INTEGER DEFAULT 0;
                "#
            )?;
        }

        // Migration: Add database column to pinned_tabs if it doesn't exist (per-tab database selection)
        let has_tab_database: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('pinned_tabs') WHERE name = 'database'",
            [],
            |row| row.get(0),
        )?;

        if !has_tab_database {
            conn.execute(
                "ALTER TABLE pinned_tabs ADD COLUMN database TEXT",
                [],
            )?;
        }

        // Migration: Add last_accessed_at column to pinned_tabs for activity tracking
        let has_last_accessed: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('pinned_tabs') WHERE name = 'last_accessed_at'",
            [],
            |row| row.get(0),
        )?;

        if !has_last_accessed {
            // Add column as nullable first (SQLite ALTER TABLE doesn't support function defaults)
            conn.execute(
                "ALTER TABLE pinned_tabs ADD COLUMN last_accessed_at TEXT",
                [],
            )?;

            // Update existing rows to set last_accessed_at to updated_at or current time
            conn.execute(
                "UPDATE pinned_tabs SET last_accessed_at = COALESCE(updated_at, datetime('now')) WHERE last_accessed_at IS NULL",
                [],
            )?;

            // Create index for auto-archive queries
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_pinned_tabs_last_accessed ON pinned_tabs(last_accessed_at, is_pinned)",
                [],
            )?;
        }

        // Migration: Create archived_tabs table
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS archived_tabs (
                id TEXT PRIMARY KEY,
                original_tab_id TEXT NOT NULL,
                space_id TEXT,
                space_name TEXT NOT NULL,
                title TEXT NOT NULL,
                tab_type TEXT NOT NULL,
                content TEXT,
                metadata TEXT,
                database TEXT,
                was_pinned INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_accessed_at TEXT NOT NULL,
                archived_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (space_id) REFERENCES spaces(id) ON DELETE SET NULL
            );

            CREATE INDEX IF NOT EXISTS idx_archived_tabs_space_id ON archived_tabs(space_id);
            CREATE INDEX IF NOT EXISTS idx_archived_tabs_archived_at ON archived_tabs(archived_at);
            CREATE INDEX IF NOT EXISTS idx_archived_tabs_last_accessed ON archived_tabs(last_accessed_at);
            "#
        )?;

        // Migration: Create tab_folders table for Arc Browser-style folder organization
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS tab_folders (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                name TEXT NOT NULL,
                is_expanded INTEGER NOT NULL DEFAULT 1,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (space_id) REFERENCES spaces(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_tab_folders_space_id ON tab_folders(space_id);
            CREATE INDEX IF NOT EXISTS idx_tab_folders_sort_order ON tab_folders(sort_order);
            "#
        )?;

        // Migration: Add folder_id column to pinned_tabs
        let has_folder_id: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('pinned_tabs') WHERE name = 'folder_id'",
            [],
            |row| row.get(0),
        )?;

        if !has_folder_id {
            // Add folder_id column
            conn.execute(
                "ALTER TABLE pinned_tabs ADD COLUMN folder_id TEXT",
                [],
            )?;

            // Create index
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_pinned_tabs_folder_id ON pinned_tabs(folder_id)",
                [],
            )?;

            // Note: Foreign key constraint (ON DELETE SET NULL) will be enforced by the existing
            // PRAGMA foreign_keys = ON setting, but SQLite's ALTER TABLE doesn't allow adding
            // constraints. New tables created from scratch would use:
            // FOREIGN KEY (folder_id) REFERENCES tab_folders(id) ON DELETE SET NULL
        }

        // Migration: Add last_active_tab_id column to spaces for remembering last opened tab per space
        let has_last_active_tab_id: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('spaces') WHERE name = 'last_active_tab_id'",
            [],
            |row| row.get(0),
        )?;

        if !has_last_active_tab_id {
            conn.execute(
                "ALTER TABLE spaces ADD COLUMN last_active_tab_id TEXT",
                [],
            )?;
        }

        // Migration: Create FTS5 virtual table for full-text search
        let has_fts: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='archived_tabs_fts'",
            [],
            |row| row.get(0),
        )?;

        if !has_fts {
            conn.execute_batch(
                r#"
                CREATE VIRTUAL TABLE archived_tabs_fts USING fts5(
                    title,
                    content,
                    content='archived_tabs',
                    content_rowid='rowid'
                );

                CREATE TRIGGER archived_tabs_fts_insert AFTER INSERT ON archived_tabs BEGIN
                    INSERT INTO archived_tabs_fts(rowid, title, content)
                    VALUES (new.rowid, new.title, COALESCE(new.content, ''));
                END;

                CREATE TRIGGER archived_tabs_fts_delete AFTER DELETE ON archived_tabs BEGIN
                    DELETE FROM archived_tabs_fts WHERE rowid = old.rowid;
                END;

                CREATE TRIGGER archived_tabs_fts_update AFTER UPDATE ON archived_tabs BEGIN
                    UPDATE archived_tabs_fts
                    SET title = new.title, content = COALESCE(new.content, '')
                    WHERE rowid = new.rowid;
                END;
                "#
            )?;
        } else {
            // Migration: Fix FTS5 table if it has the old schema with archived_tab_id
            // Check if the FTS5 table has the archived_tab_id column by trying to query it
            let needs_migration = conn.query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='archived_tabs_fts'",
                [],
                |row| {
                    let sql: String = row.get(0)?;
                    Ok(sql.contains("archived_tab_id"))
                },
            ).unwrap_or(false);

            if needs_migration {
                // Drop the old FTS5 table and triggers, then recreate
                conn.execute_batch(
                    r#"
                    DROP TRIGGER IF EXISTS archived_tabs_fts_insert;
                    DROP TRIGGER IF EXISTS archived_tabs_fts_delete;
                    DROP TRIGGER IF EXISTS archived_tabs_fts_update;
                    DROP TABLE IF EXISTS archived_tabs_fts;

                    CREATE VIRTUAL TABLE archived_tabs_fts USING fts5(
                        title,
                        content,
                        content='archived_tabs',
                        content_rowid='rowid'
                    );

                    CREATE TRIGGER archived_tabs_fts_insert AFTER INSERT ON archived_tabs BEGIN
                        INSERT INTO archived_tabs_fts(rowid, title, content)
                        VALUES (new.rowid, new.title, COALESCE(new.content, ''));
                    END;

                    CREATE TRIGGER archived_tabs_fts_delete AFTER DELETE ON archived_tabs BEGIN
                        DELETE FROM archived_tabs_fts WHERE rowid = old.rowid;
                    END;

                    CREATE TRIGGER archived_tabs_fts_update AFTER UPDATE ON archived_tabs BEGIN
                        UPDATE archived_tabs_fts
                        SET title = new.title, content = COALESCE(new.content, '')
                        WHERE rowid = new.rowid;
                    END;

                    -- Rebuild the FTS index from existing archived tabs
                    INSERT INTO archived_tabs_fts(rowid, title, content)
                    SELECT rowid, title, COALESCE(content, '') FROM archived_tabs;
                    "#
                )?;
            }
        }

        Ok(())
    }

    /// Execute a function with database connection access
    pub fn with_connection<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&Connection) -> SqliteResult<T>,
    {
        let conn = self.connection.lock().unwrap();
        f(&conn).map_err(StorageError::from)
    }

    /// Execute a function with mutable database connection access
    pub fn with_connection_mut<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&mut Connection) -> SqliteResult<T>,
    {
        let mut conn = self.connection.lock().unwrap();
        f(&mut conn).map_err(StorageError::from)
    }
}

/// Get the default database path in the app data directory
pub fn get_default_db_path() -> StorageResult<PathBuf> {
    let proj_dirs = directories::ProjectDirs::from("com", "larik", "larik-sql-studio")
        .ok_or(StorageError::AppDataDir)?;
    
    let data_dir = proj_dirs.data_dir();
    Ok(data_dir.join("larik.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_database_creation() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("larik_test.db");
        
        // Clean up any existing test database
        let _ = std::fs::remove_file(&db_path);

        let manager = DatabaseManager::new(db_path.clone()).unwrap();
        
        assert!(Path::new(&db_path).exists());
        assert_eq!(manager.db_path(), &db_path);

        // Clean up
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_schema_initialization() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("larik_schema_test.db");
        
        let _ = std::fs::remove_file(&db_path);

        let manager = DatabaseManager::new(db_path.clone()).unwrap();
        
        // Verify tables exist
        manager.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('spaces', 'pinned_tabs', 'app_state')"
            )?;
            let tables: Vec<String> = stmt
                .query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            
            assert_eq!(tables.len(), 3);
            Ok(())
        }).unwrap();

        let _ = std::fs::remove_file(&db_path);
    }
}
