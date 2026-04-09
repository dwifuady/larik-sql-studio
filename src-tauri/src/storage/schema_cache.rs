use rusqlite::params;
use serde_json;

use super::database::{DatabaseManager, StorageResult};
use crate::db::schema::SchemaInfo;

impl DatabaseManager {
    /// Save schema information for a specific connection and database
    pub fn save_schema(
        &self,
        connection_id: &str,
        database_name: &str,
        schema_info: &SchemaInfo,
    ) -> StorageResult<()> {
        let json_data = serde_json::to_string(schema_info).map_err(|e| {
            super::database::StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            ))
        })?;

        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO schema_cache (connection_id, database_name, schema_info, updated_at)
                VALUES (?1, ?2, ?3, datetime('now'))
                ON CONFLICT(connection_id, database_name) DO UPDATE SET
                    schema_info = excluded.schema_info,
                    updated_at = datetime('now')
                "#,
                params![connection_id, database_name, json_data],
            )?;
            Ok(())
        })
    }

    /// Retrieve cached schema information for a specific connection and database
    pub fn get_schema(
        &self,
        connection_id: &str,
        database_name: &str,
    ) -> StorageResult<Option<SchemaInfo>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT schema_info FROM schema_cache WHERE connection_id = ?1 AND database_name = ?2",
            )?;

            let result = stmt.query_row(params![connection_id, database_name], |row| {
                row.get::<_, String>(0)
            });

            match result {
                Ok(json_data) => {
                    let schema_info: SchemaInfo = serde_json::from_str(&json_data)
                        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                            0, rusqlite::types::Type::Text, Box::new(e)
                        ))?;
                    Ok(Some(schema_info))
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Clear schema cache entry
    pub fn clear_schema(
        &self,
        connection_id: &str,
        database_name: Option<&str>,
    ) -> StorageResult<()> {
        self.with_connection(|conn| {
            if let Some(db) = database_name {
                conn.execute(
                    "DELETE FROM schema_cache WHERE connection_id = ?1 AND database_name = ?2",
                    params![connection_id, db],
                )?;
            } else {
                conn.execute(
                    "DELETE FROM schema_cache WHERE connection_id = ?1",
                    params![connection_id],
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
            "larik_schema_cache_test_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&db_path);
        let manager = DatabaseManager::new(db_path.clone()).unwrap();
        (manager, db_path)
    }

    #[test]
    fn test_schema_cache_crud() {
        let (manager, db_path) = create_test_db();

        let schema_info = SchemaInfo {
            database_name: "testdb".to_string(),
            schemas: vec!["dbo".to_string()],
            tables: vec![],
            relationships: vec![],
            routines: vec![],
            fetched_at: "2023-01-01T00:00:00Z".to_string(),
        };

        // Cache miss
        assert!(manager.get_schema("conn1", "testdb").unwrap().is_none());

        // Save schema
        manager
            .save_schema("conn1", "testdb", &schema_info)
            .unwrap();

        // Retrieve schema
        let cached = manager.get_schema("conn1", "testdb").unwrap().unwrap();
        assert_eq!(cached.database_name, "testdb");
        assert_eq!(cached.schemas.len(), 1);

        // Clear schema
        manager.clear_schema("conn1", Some("testdb")).unwrap();
        assert!(manager.get_schema("conn1", "testdb").unwrap().is_none());

        // Save multiple, clear by connection
        manager
            .save_schema("conn1", "testdb1", &schema_info)
            .unwrap();
        manager
            .save_schema("conn1", "testdb2", &schema_info)
            .unwrap();
        manager
            .save_schema("conn2", "testdb1", &schema_info)
            .unwrap();

        manager.clear_schema("conn1", None).unwrap();
        assert!(manager.get_schema("conn1", "testdb1").unwrap().is_none());
        assert!(manager.get_schema("conn1", "testdb2").unwrap().is_none());
        assert!(manager.get_schema("conn2", "testdb1").unwrap().is_some());

        let _ = std::fs::remove_file(&db_path);
    }
}
