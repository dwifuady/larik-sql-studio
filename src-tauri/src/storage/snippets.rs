// SQL Code Snippets storage (T046)
// Handles persistence of user-defined and built-in SQL snippets

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::{DatabaseManager, StorageResult};

/// A SQL code snippet with trigger text and expansion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub id: String,
    /// The short trigger text (e.g., "sel", "ssf")
    pub trigger: String,
    /// Display name for the snippet
    pub name: String,
    /// The expanded SQL content with optional ${cursor} placeholder
    pub content: String,
    /// Optional description
    pub description: Option<String>,
    /// Whether this is a built-in snippet or user-defined
    pub is_builtin: bool,
    /// Whether the snippet is enabled
    pub enabled: bool,
    /// Category for grouping (e.g., "Select", "Insert", "DDL")
    pub category: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating a new snippet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSnippetInput {
    pub trigger: String,
    pub name: String,
    pub content: String,
    pub description: Option<String>,
    pub category: Option<String>,
}

/// Input for updating an existing snippet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSnippetInput {
    pub trigger: Option<String>,
    pub name: Option<String>,
    pub content: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
}

/// Built-in snippets that ship with the application
pub fn get_builtin_snippets() -> Vec<CreateSnippetInput> {
    vec![
        // SELECT snippets
        CreateSnippetInput {
            trigger: "sel".to_string(),
            name: "SELECT *".to_string(),
            content: "SELECT * FROM ${cursor}".to_string(),
            description: Some("Select all columns from a table".to_string()),
            category: Some("Select".to_string()),
        },
        CreateSnippetInput {
            trigger: "selt".to_string(),
            name: "SELECT TOP".to_string(),
            content: "SELECT TOP ${1:100} * FROM ${cursor}".to_string(),
            description: Some("Select top N rows from a table".to_string()),
            category: Some("Select".to_string()),
        },
        CreateSnippetInput {
            trigger: "selc".to_string(),
            name: "SELECT COUNT".to_string(),
            content: "SELECT COUNT(*) FROM ${cursor}".to_string(),
            description: Some("Count rows in a table".to_string()),
            category: Some("Select".to_string()),
        },
        CreateSnippetInput {
            trigger: "seld".to_string(),
            name: "SELECT DISTINCT".to_string(),
            content: "SELECT DISTINCT ${1:column} FROM ${cursor}".to_string(),
            description: Some("Select distinct values".to_string()),
            category: Some("Select".to_string()),
        },
        CreateSnippetInput {
            trigger: "selw".to_string(),
            name: "SELECT WHERE".to_string(),
            content: "SELECT * FROM ${1:table} WHERE ${cursor}".to_string(),
            description: Some("Select with WHERE clause".to_string()),
            category: Some("Select".to_string()),
        },
        CreateSnippetInput {
            trigger: "selj".to_string(),
            name: "SELECT JOIN".to_string(),
            content: "SELECT ${1:columns}\nFROM ${2:table1} t1\nINNER JOIN ${3:table2} t2 ON t1.${4:id} = t2.${5:id}\nWHERE ${cursor}".to_string(),
            description: Some("Select with INNER JOIN".to_string()),
            category: Some("Select".to_string()),
        },
        CreateSnippetInput {
            trigger: "sellj".to_string(),
            name: "SELECT LEFT JOIN".to_string(),
            content: "SELECT ${1:columns}\nFROM ${2:table1} t1\nLEFT JOIN ${3:table2} t2 ON t1.${4:id} = t2.${5:id}\nWHERE ${cursor}".to_string(),
            description: Some("Select with LEFT JOIN".to_string()),
            category: Some("Select".to_string()),
        },
        CreateSnippetInput {
            trigger: "selg".to_string(),
            name: "SELECT GROUP BY".to_string(),
            content: "SELECT ${1:column}, COUNT(*) AS Count\nFROM ${2:table}\nGROUP BY ${1:column}\nORDER BY Count DESC".to_string(),
            description: Some("Select with GROUP BY".to_string()),
            category: Some("Select".to_string()),
        },
        
        // INSERT snippets
        CreateSnippetInput {
            trigger: "ins".to_string(),
            name: "INSERT INTO".to_string(),
            content: "INSERT INTO ${1:table} (${2:columns})\nVALUES (${cursor})".to_string(),
            description: Some("Insert single row".to_string()),
            category: Some("Insert".to_string()),
        },
        CreateSnippetInput {
            trigger: "inss".to_string(),
            name: "INSERT SELECT".to_string(),
            content: "INSERT INTO ${1:target_table} (${2:columns})\nSELECT ${3:columns}\nFROM ${cursor}".to_string(),
            description: Some("Insert from SELECT".to_string()),
            category: Some("Insert".to_string()),
        },
        
        // UPDATE snippets
        CreateSnippetInput {
            trigger: "upd".to_string(),
            name: "UPDATE".to_string(),
            content: "UPDATE ${1:table}\nSET ${2:column} = ${3:value}\nWHERE ${cursor}".to_string(),
            description: Some("Update rows".to_string()),
            category: Some("Update".to_string()),
        },
        CreateSnippetInput {
            trigger: "updj".to_string(),
            name: "UPDATE JOIN".to_string(),
            content: "UPDATE t1\nSET t1.${1:column} = t2.${2:column}\nFROM ${3:table1} t1\nINNER JOIN ${4:table2} t2 ON t1.${5:id} = t2.${6:id}\nWHERE ${cursor}".to_string(),
            description: Some("Update with JOIN".to_string()),
            category: Some("Update".to_string()),
        },
        
        // DELETE snippets
        CreateSnippetInput {
            trigger: "del".to_string(),
            name: "DELETE".to_string(),
            content: "DELETE FROM ${1:table}\nWHERE ${cursor}".to_string(),
            description: Some("Delete rows".to_string()),
            category: Some("Delete".to_string()),
        },
        CreateSnippetInput {
            trigger: "trunc".to_string(),
            name: "TRUNCATE".to_string(),
            content: "TRUNCATE TABLE ${cursor}".to_string(),
            description: Some("Truncate table".to_string()),
            category: Some("Delete".to_string()),
        },
        
        // DDL snippets
        CreateSnippetInput {
            trigger: "ct".to_string(),
            name: "CREATE TABLE".to_string(),
            content: "CREATE TABLE ${1:table_name} (\n    ${2:id} INT IDENTITY(1,1) PRIMARY KEY,\n    ${cursor}\n)".to_string(),
            description: Some("Create new table".to_string()),
            category: Some("DDL".to_string()),
        },
        CreateSnippetInput {
            trigger: "ata".to_string(),
            name: "ALTER TABLE ADD".to_string(),
            content: "ALTER TABLE ${1:table}\nADD ${2:column} ${cursor}".to_string(),
            description: Some("Add column to table".to_string()),
            category: Some("DDL".to_string()),
        },
        CreateSnippetInput {
            trigger: "atd".to_string(),
            name: "ALTER TABLE DROP".to_string(),
            content: "ALTER TABLE ${1:table}\nDROP COLUMN ${cursor}".to_string(),
            description: Some("Drop column from table".to_string()),
            category: Some("DDL".to_string()),
        },
        CreateSnippetInput {
            trigger: "ci".to_string(),
            name: "CREATE INDEX".to_string(),
            content: "CREATE INDEX IX_${1:table}_${2:column}\nON ${1:table} (${2:column})".to_string(),
            description: Some("Create index".to_string()),
            category: Some("DDL".to_string()),
        },
        CreateSnippetInput {
            trigger: "di".to_string(),
            name: "DROP INDEX".to_string(),
            content: "DROP INDEX ${1:index_name} ON ${cursor}".to_string(),
            description: Some("Drop index".to_string()),
            category: Some("DDL".to_string()),
        },
        
        // CTE snippets
        CreateSnippetInput {
            trigger: "cte".to_string(),
            name: "CTE".to_string(),
            content: ";WITH ${1:cte_name} AS (\n    ${2:SELECT * FROM table}\n)\nSELECT * FROM ${1:cte_name}${cursor}".to_string(),
            description: Some("Common Table Expression".to_string()),
            category: Some("CTE".to_string()),
        },
        CreateSnippetInput {
            trigger: "rcte".to_string(),
            name: "Recursive CTE".to_string(),
            content: ";WITH ${1:cte_name} AS (\n    -- Anchor\n    SELECT ${2:columns}\n    FROM ${3:table}\n    WHERE ${4:condition}\n    \n    UNION ALL\n    \n    -- Recursive\n    SELECT ${2:columns}\n    FROM ${3:table} t\n    INNER JOIN ${1:cte_name} c ON t.${5:parent_id} = c.${6:id}\n)\nSELECT * FROM ${1:cte_name}${cursor}".to_string(),
            description: Some("Recursive CTE".to_string()),
            category: Some("CTE".to_string()),
        },
        
        // Procedure/Function snippets
        CreateSnippetInput {
            trigger: "cp".to_string(),
            name: "CREATE PROCEDURE".to_string(),
            content: "CREATE PROCEDURE ${1:procedure_name}\n    @${2:param} ${3:INT}\nAS\nBEGIN\n    SET NOCOUNT ON;\n    ${cursor}\nEND".to_string(),
            description: Some("Create stored procedure".to_string()),
            category: Some("Procedure".to_string()),
        },
        CreateSnippetInput {
            trigger: "ap".to_string(),
            name: "ALTER PROCEDURE".to_string(),
            content: "ALTER PROCEDURE ${cursor}".to_string(),
            description: Some("Alter stored procedure".to_string()),
            category: Some("Procedure".to_string()),
        },
        CreateSnippetInput {
            trigger: "exec".to_string(),
            name: "EXEC".to_string(),
            content: "EXEC ${1:procedure_name} ${cursor}".to_string(),
            description: Some("Execute stored procedure".to_string()),
            category: Some("Procedure".to_string()),
        },
        
        // Transaction snippets
        CreateSnippetInput {
            trigger: "tran".to_string(),
            name: "Transaction".to_string(),
            content: "BEGIN TRANSACTION;\nBEGIN TRY\n    ${cursor}\n    COMMIT TRANSACTION;\nEND TRY\nBEGIN CATCH\n    ROLLBACK TRANSACTION;\n    THROW;\nEND CATCH".to_string(),
            description: Some("Transaction with error handling".to_string()),
            category: Some("Transaction".to_string()),
        },
        CreateSnippetInput {
            trigger: "tc".to_string(),
            name: "TRY CATCH".to_string(),
            content: "BEGIN TRY\n    ${cursor}\nEND TRY\nBEGIN CATCH\n    SELECT ERROR_MESSAGE() AS ErrorMessage;\nEND CATCH".to_string(),
            description: Some("Try-Catch block".to_string()),
            category: Some("Transaction".to_string()),
        },
        
        // Variable snippets
        CreateSnippetInput {
            trigger: "decl".to_string(),
            name: "DECLARE".to_string(),
            content: "DECLARE @${1:variable} ${2:INT} = ${cursor}".to_string(),
            description: Some("Declare variable".to_string()),
            category: Some("Variable".to_string()),
        },
        CreateSnippetInput {
            trigger: "dect".to_string(),
            name: "DECLARE TABLE".to_string(),
            content: "DECLARE @${1:table} TABLE (\n    ${2:id} INT,\n    ${cursor}\n)".to_string(),
            description: Some("Declare table variable".to_string()),
            category: Some("Variable".to_string()),
        },
        
        // Utility snippets
        CreateSnippetInput {
            trigger: "iff".to_string(),
            name: "IF EXISTS".to_string(),
            content: "IF EXISTS (SELECT 1 FROM ${1:table} WHERE ${2:condition})\nBEGIN\n    ${cursor}\nEND".to_string(),
            description: Some("If exists check".to_string()),
            category: Some("Control Flow".to_string()),
        },
        CreateSnippetInput {
            trigger: "ifn".to_string(),
            name: "IF NOT EXISTS".to_string(),
            content: "IF NOT EXISTS (SELECT 1 FROM ${1:table} WHERE ${2:condition})\nBEGIN\n    ${cursor}\nEND".to_string(),
            description: Some("If not exists check".to_string()),
            category: Some("Control Flow".to_string()),
        },
        CreateSnippetInput {
            trigger: "wh".to_string(),
            name: "WHILE".to_string(),
            content: "WHILE ${1:condition}\nBEGIN\n    ${cursor}\nEND".to_string(),
            description: Some("While loop".to_string()),
            category: Some("Control Flow".to_string()),
        },
        CreateSnippetInput {
            trigger: "case".to_string(),
            name: "CASE".to_string(),
            content: "CASE\n    WHEN ${1:condition} THEN ${2:result}\n    ELSE ${3:default}\nEND".to_string(),
            description: Some("CASE expression".to_string()),
            category: Some("Expression".to_string()),
        },
        
        // Info snippets
        CreateSnippetInput {
            trigger: "cols".to_string(),
            name: "Column Info".to_string(),
            content: "SELECT c.name AS ColumnName, t.name AS DataType, c.max_length, c.is_nullable\nFROM sys.columns c\nINNER JOIN sys.types t ON c.user_type_id = t.user_type_id\nWHERE c.object_id = OBJECT_ID('${cursor}')\nORDER BY c.column_id".to_string(),
            description: Some("Get column info for table".to_string()),
            category: Some("Info".to_string()),
        },
        CreateSnippetInput {
            trigger: "tbls".to_string(),
            name: "List Tables".to_string(),
            content: "SELECT TABLE_SCHEMA, TABLE_NAME\nFROM INFORMATION_SCHEMA.TABLES\nWHERE TABLE_TYPE = 'BASE TABLE'\nORDER BY TABLE_SCHEMA, TABLE_NAME".to_string(),
            description: Some("List all tables".to_string()),
            category: Some("Info".to_string()),
        },
        CreateSnippetInput {
            trigger: "idxs".to_string(),
            name: "Index Info".to_string(),
            content: "SELECT i.name AS IndexName, i.type_desc, c.name AS ColumnName\nFROM sys.indexes i\nINNER JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id\nINNER JOIN sys.columns c ON ic.object_id = c.object_id AND ic.column_id = c.column_id\nWHERE i.object_id = OBJECT_ID('${cursor}')\nORDER BY i.name, ic.key_ordinal".to_string(),
            description: Some("Get index info for table".to_string()),
            category: Some("Info".to_string()),
        },
    ]
}

impl DatabaseManager {
    /// Initialize the snippets table schema
    pub fn init_snippets_schema(&self) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute_batch(
                r#"
                -- Snippets table: SQL code snippets with triggers
                CREATE TABLE IF NOT EXISTS snippets (
                    id TEXT PRIMARY KEY,
                    trigger TEXT NOT NULL,
                    name TEXT NOT NULL,
                    content TEXT NOT NULL,
                    description TEXT,
                    is_builtin INTEGER NOT NULL DEFAULT 0,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    category TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                -- Index for quick trigger lookups
                CREATE INDEX IF NOT EXISTS idx_snippets_trigger ON snippets(trigger);
                CREATE INDEX IF NOT EXISTS idx_snippets_enabled ON snippets(enabled);
                "#,
            )?;
            Ok(())
        })
    }

    /// Seed built-in snippets if they don't exist
    pub fn seed_builtin_snippets(&self) -> StorageResult<usize> {
        let builtins = get_builtin_snippets();
        let mut count = 0;

        for snippet in builtins {
            // Check if a builtin with this trigger already exists
            let exists: bool = self.with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) > 0 FROM snippets WHERE trigger = ?1 AND is_builtin = 1",
                    params![&snippet.trigger],
                    |row| row.get(0),
                )
            })?;

            if !exists {
                self.create_snippet_internal(
                    &snippet.trigger,
                    &snippet.name,
                    &snippet.content,
                    snippet.description.as_deref(),
                    snippet.category.as_deref(),
                    true, // is_builtin
                )?;
                count += 1;
            }
        }

        Ok(count)
    }

    /// Create a new snippet (internal - with is_builtin flag)
    fn create_snippet_internal(
        &self,
        trigger: &str,
        name: &str,
        content: &str,
        description: Option<&str>,
        category: Option<&str>,
        is_builtin: bool,
    ) -> StorageResult<Snippet> {
        let id = Uuid::new_v4().to_string();
        
        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO snippets (id, trigger, name, content, description, is_builtin, enabled, category)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)
                "#,
                params![id, trigger, name, content, description, is_builtin, category],
            )?;
            
            conn.query_row(
                "SELECT id, trigger, name, content, description, is_builtin, enabled, category, created_at, updated_at FROM snippets WHERE id = ?1",
                params![id],
                |row| Ok(Snippet {
                    id: row.get(0)?,
                    trigger: row.get(1)?,
                    name: row.get(2)?,
                    content: row.get(3)?,
                    description: row.get(4)?,
                    is_builtin: row.get::<_, i32>(5)? != 0,
                    enabled: row.get::<_, i32>(6)? != 0,
                    category: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                }),
            )
        })
    }

    /// Create a new user-defined snippet
    pub fn create_snippet(&self, input: CreateSnippetInput) -> StorageResult<Snippet> {
        self.create_snippet_internal(
            &input.trigger,
            &input.name,
            &input.content,
            input.description.as_deref(),
            input.category.as_deref(),
            false, // is_builtin = false for user-defined
        )
    }

    /// Get all snippets
    pub fn get_all_snippets(&self) -> StorageResult<Vec<Snippet>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, trigger, name, content, description, is_builtin, enabled, category, created_at, updated_at 
                 FROM snippets 
                 ORDER BY category NULLS LAST, trigger",
            )?;
            
            let snippets = stmt
                .query_map([], |row| {
                    Ok(Snippet {
                        id: row.get(0)?,
                        trigger: row.get(1)?,
                        name: row.get(2)?,
                        content: row.get(3)?,
                        description: row.get(4)?,
                        is_builtin: row.get::<_, i32>(5)? != 0,
                        enabled: row.get::<_, i32>(6)? != 0,
                        category: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            
            Ok(snippets)
        })
    }

    /// Get only enabled snippets
    pub fn get_enabled_snippets(&self) -> StorageResult<Vec<Snippet>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, trigger, name, content, description, is_builtin, enabled, category, created_at, updated_at 
                 FROM snippets 
                 WHERE enabled = 1
                 ORDER BY category NULLS LAST, trigger",
            )?;
            
            let snippets = stmt
                .query_map([], |row| {
                    Ok(Snippet {
                        id: row.get(0)?,
                        trigger: row.get(1)?,
                        name: row.get(2)?,
                        content: row.get(3)?,
                        description: row.get(4)?,
                        is_builtin: row.get::<_, i32>(5)? != 0,
                        enabled: row.get::<_, i32>(6)? != 0,
                        category: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            
            Ok(snippets)
        })
    }

    /// Get a snippet by ID
    pub fn get_snippet(&self, id: &str) -> StorageResult<Option<Snippet>> {
        self.with_connection(|conn| {
            let result = conn.query_row(
                "SELECT id, trigger, name, content, description, is_builtin, enabled, category, created_at, updated_at 
                 FROM snippets WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Snippet {
                        id: row.get(0)?,
                        trigger: row.get(1)?,
                        name: row.get(2)?,
                        content: row.get(3)?,
                        description: row.get(4)?,
                        is_builtin: row.get::<_, i32>(5)? != 0,
                        enabled: row.get::<_, i32>(6)? != 0,
                        category: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                },
            );
            
            match result {
                Ok(snippet) => Ok(Some(snippet)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Get a snippet by trigger text
    pub fn get_snippet_by_trigger(&self, trigger: &str) -> StorageResult<Option<Snippet>> {
        self.with_connection(|conn| {
            let result = conn.query_row(
                "SELECT id, trigger, name, content, description, is_builtin, enabled, category, created_at, updated_at 
                 FROM snippets WHERE trigger = ?1 AND enabled = 1",
                params![trigger],
                |row| {
                    Ok(Snippet {
                        id: row.get(0)?,
                        trigger: row.get(1)?,
                        name: row.get(2)?,
                        content: row.get(3)?,
                        description: row.get(4)?,
                        is_builtin: row.get::<_, i32>(5)? != 0,
                        enabled: row.get::<_, i32>(6)? != 0,
                        category: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                },
            );
            
            match result {
                Ok(snippet) => Ok(Some(snippet)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Update an existing snippet
    pub fn update_snippet(&self, id: &str, input: UpdateSnippetInput) -> StorageResult<Option<Snippet>> {
        // First check if snippet exists and is not builtin (if trying to change trigger/name/content)
        let existing = self.get_snippet(id)?;
        if existing.is_none() {
            return Ok(None);
        }
        
        self.with_connection(|conn| {
            let mut updates = Vec::new();
            let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            
            if let Some(ref trigger) = input.trigger {
                updates.push("trigger = ?");
                values.push(Box::new(trigger.clone()));
            }
            if let Some(ref name) = input.name {
                updates.push("name = ?");
                values.push(Box::new(name.clone()));
            }
            if let Some(ref content) = input.content {
                updates.push("content = ?");
                values.push(Box::new(content.clone()));
            }
            if let Some(ref description) = input.description {
                updates.push("description = ?");
                values.push(Box::new(description.clone()));
            }
            if let Some(ref category) = input.category {
                updates.push("category = ?");
                values.push(Box::new(category.clone()));
            }
            if let Some(enabled) = input.enabled {
                updates.push("enabled = ?");
                values.push(Box::new(enabled as i32));
            }
            
            if updates.is_empty() {
                return Ok(Some(existing.unwrap()));
            }
            
            updates.push("updated_at = datetime('now')");
            values.push(Box::new(id.to_string()));
            
            let sql = format!(
                "UPDATE snippets SET {} WHERE id = ?",
                updates.join(", ")
            );
            
            let params: Vec<&dyn rusqlite::ToSql> = values.iter().map(|v| v.as_ref()).collect();
            conn.execute(&sql, params.as_slice())?;
            
            // Fetch and return updated snippet
            conn.query_row(
                "SELECT id, trigger, name, content, description, is_builtin, enabled, category, created_at, updated_at 
                 FROM snippets WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Some(Snippet {
                        id: row.get(0)?,
                        trigger: row.get(1)?,
                        name: row.get(2)?,
                        content: row.get(3)?,
                        description: row.get(4)?,
                        is_builtin: row.get::<_, i32>(5)? != 0,
                        enabled: row.get::<_, i32>(6)? != 0,
                        category: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    }))
                },
            )
        })
    }

    /// Delete a snippet (only user-defined snippets can be deleted)
    pub fn delete_snippet(&self, id: &str) -> StorageResult<bool> {
        self.with_connection(|conn| {
            // Only delete if it's not a builtin
            let rows = conn.execute(
                "DELETE FROM snippets WHERE id = ?1 AND is_builtin = 0",
                params![id],
            )?;
            Ok(rows > 0)
        })
    }

    /// Reset a builtin snippet to its default content
    pub fn reset_builtin_snippet(&self, id: &str) -> StorageResult<Option<Snippet>> {
        let snippet = self.get_snippet(id)?;
        if let Some(snippet) = snippet {
            if !snippet.is_builtin {
                return Ok(None);
            }
            
            // Find the original builtin
            let builtins = get_builtin_snippets();
            if let Some(original) = builtins.iter().find(|s| s.trigger == snippet.trigger) {
                return self.update_snippet(id, UpdateSnippetInput {
                    trigger: None,
                    name: Some(original.name.clone()),
                    content: Some(original.content.clone()),
                    description: original.description.clone(),
                    category: original.category.clone(),
                    enabled: Some(true),
                });
            }
        }
        Ok(None)
    }

    /// Import snippets from external source (bulk import)
    pub fn import_snippets(&self, snippets: Vec<CreateSnippetInput>) -> StorageResult<usize> {
        let mut count = 0;
        for snippet in snippets {
            // Check if trigger already exists
            let exists = self.get_snippet_by_trigger(&snippet.trigger)?.is_some();
            if !exists {
                self.create_snippet(snippet)?;
                count += 1;
            }
        }
        Ok(count)
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
        let db_path = temp_dir.join(format!("larik_snippet_test_{}_{}.db", std::process::id(), counter));
        let _ = std::fs::remove_file(&db_path);
        let manager = DatabaseManager::new(db_path.clone()).unwrap();
        manager.init_snippets_schema().unwrap();
        (manager, db_path)
    }

    #[test]
    fn test_create_and_get_snippet() {
        let (manager, db_path) = create_test_db();

        let snippet = manager.create_snippet(CreateSnippetInput {
            trigger: "test".to_string(),
            name: "Test Snippet".to_string(),
            content: "SELECT * FROM ${cursor}".to_string(),
            description: Some("A test snippet".to_string()),
            category: Some("Test".to_string()),
        }).unwrap();

        assert_eq!(snippet.trigger, "test");
        assert_eq!(snippet.name, "Test Snippet");
        assert!(!snippet.is_builtin);
        assert!(snippet.enabled);

        let fetched = manager.get_snippet(&snippet.id).unwrap().unwrap();
        assert_eq!(fetched.id, snippet.id);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_seed_builtin_snippets() {
        let (manager, db_path) = create_test_db();

        let count = manager.seed_builtin_snippets().unwrap();
        assert!(count > 0);

        // Running again should not add duplicates
        let count2 = manager.seed_builtin_snippets().unwrap();
        assert_eq!(count2, 0);

        let snippets = manager.get_all_snippets().unwrap();
        assert_eq!(snippets.len(), count);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_get_snippet_by_trigger() {
        let (manager, db_path) = create_test_db();
        manager.seed_builtin_snippets().unwrap();

        let snippet = manager.get_snippet_by_trigger("sel").unwrap();
        assert!(snippet.is_some());
        assert_eq!(snippet.unwrap().trigger, "sel");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_delete_user_snippet() {
        let (manager, db_path) = create_test_db();

        let snippet = manager.create_snippet(CreateSnippetInput {
            trigger: "usrsnp".to_string(),
            name: "User Snippet".to_string(),
            content: "test".to_string(),
            description: None,
            category: None,
        }).unwrap();

        let deleted = manager.delete_snippet(&snippet.id).unwrap();
        assert!(deleted);

        let fetched = manager.get_snippet(&snippet.id).unwrap();
        assert!(fetched.is_none());

        let _ = std::fs::remove_file(&db_path);
    }
}
