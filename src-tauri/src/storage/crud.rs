// Basic CRUD operations for app state key-value storage

use rusqlite::params;
use super::database::{DatabaseManager, StorageResult};

impl DatabaseManager {
    /// Get a value from the app state store
    pub fn get_state(&self, key: &str) -> StorageResult<Option<String>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT value FROM app_state WHERE key = ?1")?;
            let result = stmt.query_row(params![key], |row| row.get(0));
            
            match result {
                Ok(value) => Ok(Some(value)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Set a value in the app state store
    pub fn set_state(&self, key: &str, value: &str) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO app_state (key, value, updated_at) 
                VALUES (?1, ?2, datetime('now'))
                ON CONFLICT(key) DO UPDATE SET 
                    value = excluded.value,
                    updated_at = datetime('now')
                "#,
                params![key, value],
            )?;
            Ok(())
        })
    }

    /// Delete a value from the app state store
    pub fn delete_state(&self, key: &str) -> StorageResult<bool> {
        self.with_connection(|conn| {
            let rows_affected = conn.execute(
                "DELETE FROM app_state WHERE key = ?1",
                params![key],
            )?;
            Ok(rows_affected > 0)
        })
    }

    /// Get all app state keys
    pub fn get_all_state_keys(&self) -> StorageResult<Vec<String>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT key FROM app_state ORDER BY key")?;
            let keys = stmt
                .query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(keys)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    
    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn create_test_db() -> (DatabaseManager, PathBuf) {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("larik_crud_test_{}_{}.db", std::process::id(), counter));
        let _ = std::fs::remove_file(&db_path);
        let manager = DatabaseManager::new(db_path.clone()).unwrap();
        (manager, db_path)
    }

    #[test]
    fn test_set_and_get_state() {
        let (manager, db_path) = create_test_db();

        manager.set_state("test_key", "test_value").unwrap();
        let value = manager.get_state("test_key").unwrap();
        
        assert_eq!(value, Some("test_value".to_string()));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_get_nonexistent_state() {
        let (manager, db_path) = create_test_db();

        let value = manager.get_state("nonexistent").unwrap();
        assert_eq!(value, None);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_update_state() {
        let (manager, db_path) = create_test_db();

        manager.set_state("key", "value1").unwrap();
        manager.set_state("key", "value2").unwrap();
        
        let value = manager.get_state("key").unwrap();
        assert_eq!(value, Some("value2".to_string()));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_delete_state() {
        let (manager, db_path) = create_test_db();

        manager.set_state("key", "value").unwrap();
        let deleted = manager.delete_state("key").unwrap();
        
        assert!(deleted);
        assert_eq!(manager.get_state("key").unwrap(), None);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_get_all_state_keys() {
        let (manager, db_path) = create_test_db();

        manager.set_state("alpha", "1").unwrap();
        manager.set_state("beta", "2").unwrap();
        manager.set_state("gamma", "3").unwrap();

        let keys = manager.get_all_state_keys().unwrap();
        assert_eq!(keys, vec!["alpha", "beta", "gamma"]);

        let _ = std::fs::remove_file(&db_path);
    }
}
