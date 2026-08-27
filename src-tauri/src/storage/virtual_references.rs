// Virtual (user-defined) foreign key storage.
//
// Some databases model a lookup relationship without declaring a FOREIGN KEY.
// A virtual reference lets the user point a column at its lookup table by hand
// so the reference preview works the same way it does for a real constraint.
// These live only in Larik's local database, scoped to connection + database.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::database::{DatabaseManager, StorageResult};

/// A user-defined column -> table reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualReference {
    pub id: String,
    pub connection_id: String,
    pub database_name: String,
    pub source_schema: String,
    pub source_table: String,
    pub source_column: String,
    pub target_schema: String,
    pub target_table: String,
    pub target_column: String,
    pub created_at: String,
    pub updated_at: String,
}

impl DatabaseManager {
    /// All virtual references defined for a connection + database.
    pub fn get_virtual_references(
        &self,
        connection_id: &str,
        database_name: &str,
    ) -> StorageResult<Vec<VirtualReference>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, connection_id, database_name,
                        source_schema, source_table, source_column,
                        target_schema, target_table, target_column,
                        created_at, updated_at
                 FROM virtual_references
                 WHERE connection_id = ?1 AND database_name = ?2 COLLATE NOCASE
                 ORDER BY source_table, source_column",
            )?;

            let references = stmt
                .query_map(params![connection_id, database_name], |row| {
                    Ok(VirtualReference {
                        id: row.get(0)?,
                        connection_id: row.get(1)?,
                        database_name: row.get(2)?,
                        source_schema: row.get(3)?,
                        source_table: row.get(4)?,
                        source_column: row.get(5)?,
                        target_schema: row.get(6)?,
                        target_table: row.get(7)?,
                        target_column: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(references)
        })
    }

    /// Save a virtual reference, replacing any existing one for the same source
    /// column. Identifier comparisons are case-insensitive, matching SQL Server.
    pub fn save_virtual_reference(&self, reference: &VirtualReference) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                "DELETE FROM virtual_references
                 WHERE connection_id = ?1
                   AND database_name = ?2 COLLATE NOCASE
                   AND source_schema = ?3 COLLATE NOCASE
                   AND source_table = ?4 COLLATE NOCASE
                   AND source_column = ?5 COLLATE NOCASE",
                params![
                    reference.connection_id,
                    reference.database_name,
                    reference.source_schema,
                    reference.source_table,
                    reference.source_column,
                ],
            )?;

            conn.execute(
                "INSERT INTO virtual_references
                    (id, connection_id, database_name,
                     source_schema, source_table, source_column,
                     target_schema, target_table, target_column,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    reference.id,
                    reference.connection_id,
                    reference.database_name,
                    reference.source_schema,
                    reference.source_table,
                    reference.source_column,
                    reference.target_schema,
                    reference.target_table,
                    reference.target_column,
                    reference.created_at,
                    reference.updated_at,
                ],
            )?;

            Ok(())
        })
    }

    /// Delete a virtual reference by id.
    pub fn delete_virtual_reference(&self, id: &str) -> StorageResult<bool> {
        self.with_connection(|conn| {
            let affected = conn.execute("DELETE FROM virtual_references WHERE id = ?1", params![id])?;
            Ok(affected > 0)
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn create_test_db() -> (DatabaseManager, PathBuf) {
        let db_path = std::env::temp_dir().join(format!("larik_vref_test_{}.db", Uuid::new_v4()));
        let manager = DatabaseManager::new(db_path.clone()).unwrap();
        (manager, db_path)
    }

    fn sample(id: &str, source_column: &str, target_table: &str) -> VirtualReference {
        VirtualReference {
            id: id.to_string(),
            connection_id: "space1".to_string(),
            database_name: "AppDb".to_string(),
            source_schema: "dbo".to_string(),
            source_table: "Transaction".to_string(),
            source_column: source_column.to_string(),
            target_schema: "dbo".to_string(),
            target_table: target_table.to_string(),
            target_column: "Id".to_string(),
            created_at: "2026-07-30T00:00:00Z".to_string(),
            updated_at: "2026-07-30T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_virtual_reference_crud() {
        let (db, db_path) = create_test_db();

        assert!(db.get_virtual_references("space1", "AppDb").unwrap().is_empty());

        db.save_virtual_reference(&sample("v1", "Status", "TransactionStatus")).unwrap();
        db.save_virtual_reference(&sample("v2", "UserId", "User")).unwrap();

        let all = db.get_virtual_references("space1", "AppDb").unwrap();
        assert_eq!(all.len(), 2);

        // Scoped by connection and database
        assert!(db.get_virtual_references("space2", "AppDb").unwrap().is_empty());
        assert!(db.get_virtual_references("space1", "OtherDb").unwrap().is_empty());

        // Database name matching is case-insensitive
        assert_eq!(db.get_virtual_references("space1", "appdb").unwrap().len(), 2);

        // Saving the same source column again replaces the previous entry,
        // including when the identifiers differ only in case
        db.save_virtual_reference(&sample("v3", "status", "ApplicationStatus")).unwrap();
        let all = db.get_virtual_references("space1", "AppDb").unwrap();
        assert_eq!(all.len(), 2);
        let status = all.iter().find(|r| r.source_column.eq_ignore_ascii_case("status")).unwrap();
        assert_eq!(status.target_table, "ApplicationStatus");
        assert_eq!(status.id, "v3");

        assert!(db.delete_virtual_reference("v3").unwrap());
        assert!(!db.delete_virtual_reference("v3").unwrap());
        assert_eq!(db.get_virtual_references("space1", "AppDb").unwrap().len(), 1);

        let _ = std::fs::remove_file(&db_path);
    }
}
