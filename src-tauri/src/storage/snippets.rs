// SQL Code Snippets storage (T046)
// Handles persistence of user-defined and built-in SQL snippets

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::{DatabaseManager, StorageResult};
use crate::db::driver::DriverKind;

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
    /// SQL drivers the snippet applies to (`DriverKind` names); `None` = every SQL database
    pub drivers: Option<Vec<String>>,
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
    /// SQL drivers the snippet applies to; `None`/empty = every SQL database
    #[serde(default)]
    pub drivers: Option<Vec<String>>,
}

/// Input for updating an existing snippet
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSnippetInput {
    pub trigger: Option<String>,
    pub name: Option<String>,
    pub content: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
    /// New driver scope; `Some(empty)` resets it to every SQL database
    #[serde(default)]
    pub drivers: Option<Vec<String>>,
}

/// Columns read into a `Snippet`, in `row_to_snippet` order.
const SNIPPET_COLUMNS: &str =
    "id, trigger, name, content, description, is_builtin, enabled, category, drivers, created_at, updated_at";

fn row_to_snippet(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snippet> {
    Ok(Snippet {
        id: row.get(0)?,
        trigger: row.get(1)?,
        name: row.get(2)?,
        content: row.get(3)?,
        description: row.get(4)?,
        is_builtin: row.get::<_, i32>(5)? != 0,
        enabled: row.get::<_, i32>(6)? != 0,
        category: row.get(7)?,
        drivers: row
            .get::<_, Option<String>>(8)?
            .map(|s| s.split(',').map(str::to_string).collect()),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

/// Canonical storage form of a driver scope: known SQL driver names, sorted
/// and comma-joined; `None` when the scope is empty (= every SQL database).
fn drivers_column(drivers: Option<&[String]>) -> Option<String> {
    let mut names: Vec<&'static str> = drivers
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d.parse::<DriverKind>().ok())
        .filter(|kind| *kind != DriverKind::Mongodb)
        .map(|kind| kind.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    (!names.is_empty()).then(|| names.join(","))
}

/// Built-in scoped to SQL Server.
fn mssql_only() -> Option<Vec<String>> {
    Some(vec![DriverKind::Mssql.as_str().to_string()])
}

/// Built-in scoped to PostgreSQL.
fn postgres_only() -> Option<Vec<String>> {
    Some(vec![DriverKind::Postgres.as_str().to_string()])
}

/// Built-in snippets that ship with the application. A trigger may appear
/// once per database (e.g. `selt` is `SELECT TOP` on SQL Server and
/// `... LIMIT` on PostgreSQL); `drivers: None` means every SQL database.
pub fn get_builtin_snippets() -> Vec<CreateSnippetInput> {
    vec![
        // SELECT snippets
        CreateSnippetInput {
            trigger: "sel".to_string(),
            name: "SELECT *".to_string(),
            content: "SELECT * FROM ${cursor}".to_string(),
            description: Some("Select all columns from a table".to_string()),
            category: Some("Select".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "selt".to_string(),
            name: "SELECT TOP".to_string(),
            content: "SELECT TOP ${1:100} * FROM ${cursor}".to_string(),
            description: Some("Select top N rows from a table".to_string()),
            category: Some("Select".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "selc".to_string(),
            name: "SELECT COUNT".to_string(),
            content: "SELECT COUNT(*) FROM ${cursor}".to_string(),
            description: Some("Count rows in a table".to_string()),
            category: Some("Select".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "seld".to_string(),
            name: "SELECT DISTINCT".to_string(),
            content: "SELECT DISTINCT ${1:column} FROM ${cursor}".to_string(),
            description: Some("Select distinct values".to_string()),
            category: Some("Select".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "selw".to_string(),
            name: "SELECT WHERE".to_string(),
            content: "SELECT * FROM ${1:table} WHERE ${cursor}".to_string(),
            description: Some("Select with WHERE clause".to_string()),
            category: Some("Select".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "selj".to_string(),
            name: "SELECT JOIN".to_string(),
            content: "SELECT ${1:columns}\nFROM ${2:table1} t1\nINNER JOIN ${3:table2} t2 ON t1.${4:id} = t2.${5:id}\nWHERE ${cursor}".to_string(),
            description: Some("Select with INNER JOIN".to_string()),
            category: Some("Select".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "sellj".to_string(),
            name: "SELECT LEFT JOIN".to_string(),
            content: "SELECT ${1:columns}\nFROM ${2:table1} t1\nLEFT JOIN ${3:table2} t2 ON t1.${4:id} = t2.${5:id}\nWHERE ${cursor}".to_string(),
            description: Some("Select with LEFT JOIN".to_string()),
            category: Some("Select".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "selg".to_string(),
            name: "SELECT GROUP BY".to_string(),
            content: "SELECT ${1:column}, COUNT(*) AS Count\nFROM ${2:table}\nGROUP BY ${1:column}\nORDER BY Count DESC".to_string(),
            description: Some("Select with GROUP BY".to_string()),
            category: Some("Select".to_string()),
            drivers: None,
        },

        // INSERT snippets
        CreateSnippetInput {
            trigger: "ins".to_string(),
            name: "INSERT INTO".to_string(),
            content: "INSERT INTO ${1:table} (${2:columns})\nVALUES (${cursor})".to_string(),
            description: Some("Insert single row".to_string()),
            category: Some("Insert".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "inss".to_string(),
            name: "INSERT SELECT".to_string(),
            content: "INSERT INTO ${1:target_table} (${2:columns})\nSELECT ${3:columns}\nFROM ${cursor}".to_string(),
            description: Some("Insert from SELECT".to_string()),
            category: Some("Insert".to_string()),
            drivers: None,
        },

        // UPDATE snippets
        CreateSnippetInput {
            trigger: "upd".to_string(),
            name: "UPDATE".to_string(),
            content: "UPDATE ${1:table}\nSET ${2:column} = ${3:value}\nWHERE ${cursor}".to_string(),
            description: Some("Update rows".to_string()),
            category: Some("Update".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "updj".to_string(),
            name: "UPDATE JOIN".to_string(),
            content: "UPDATE t1\nSET t1.${1:column} = t2.${2:column}\nFROM ${3:table1} t1\nINNER JOIN ${4:table2} t2 ON t1.${5:id} = t2.${6:id}\nWHERE ${cursor}".to_string(),
            description: Some("Update with JOIN".to_string()),
            category: Some("Update".to_string()),
            drivers: mssql_only(),
        },

        // DELETE snippets
        CreateSnippetInput {
            trigger: "del".to_string(),
            name: "DELETE".to_string(),
            content: "DELETE FROM ${1:table}\nWHERE ${cursor}".to_string(),
            description: Some("Delete rows".to_string()),
            category: Some("Delete".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "trunc".to_string(),
            name: "TRUNCATE".to_string(),
            content: "TRUNCATE TABLE ${cursor}".to_string(),
            description: Some("Truncate table".to_string()),
            category: Some("Delete".to_string()),
            drivers: None,
        },

        // DDL snippets
        CreateSnippetInput {
            trigger: "ct".to_string(),
            name: "CREATE TABLE".to_string(),
            content: "CREATE TABLE ${1:table_name} (\n    ${2:id} INT IDENTITY(1,1) PRIMARY KEY,\n    ${cursor}\n)".to_string(),
            description: Some("Create new table".to_string()),
            category: Some("DDL".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "ata".to_string(),
            name: "ALTER TABLE ADD".to_string(),
            content: "ALTER TABLE ${1:table}\nADD ${2:column} ${cursor}".to_string(),
            description: Some("Add column to table".to_string()),
            category: Some("DDL".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "atd".to_string(),
            name: "ALTER TABLE DROP".to_string(),
            content: "ALTER TABLE ${1:table}\nDROP COLUMN ${cursor}".to_string(),
            description: Some("Drop column from table".to_string()),
            category: Some("DDL".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "ci".to_string(),
            name: "CREATE INDEX".to_string(),
            content: "CREATE INDEX IX_${1:table}_${2:column}\nON ${1:table} (${2:column})".to_string(),
            description: Some("Create index".to_string()),
            category: Some("DDL".to_string()),
            drivers: None,
        },
        CreateSnippetInput {
            trigger: "di".to_string(),
            name: "DROP INDEX".to_string(),
            content: "DROP INDEX ${1:index_name} ON ${cursor}".to_string(),
            description: Some("Drop index".to_string()),
            category: Some("DDL".to_string()),
            drivers: mssql_only(),
        },

        // CTE snippets
        CreateSnippetInput {
            trigger: "cte".to_string(),
            name: "CTE".to_string(),
            content: ";WITH ${1:cte_name} AS (\n    ${2:SELECT * FROM table}\n)\nSELECT * FROM ${1:cte_name}${cursor}".to_string(),
            description: Some("Common Table Expression".to_string()),
            category: Some("CTE".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "rcte".to_string(),
            name: "Recursive CTE".to_string(),
            content: ";WITH ${1:cte_name} AS (\n    -- Anchor\n    SELECT ${2:columns}\n    FROM ${3:table}\n    WHERE ${4:condition}\n    \n    UNION ALL\n    \n    -- Recursive\n    SELECT ${2:columns}\n    FROM ${3:table} t\n    INNER JOIN ${1:cte_name} c ON t.${5:parent_id} = c.${6:id}\n)\nSELECT * FROM ${1:cte_name}${cursor}".to_string(),
            description: Some("Recursive CTE".to_string()),
            category: Some("CTE".to_string()),
            drivers: mssql_only(),
        },

        // Procedure/Function snippets
        CreateSnippetInput {
            trigger: "cp".to_string(),
            name: "CREATE PROCEDURE".to_string(),
            content: "CREATE PROCEDURE ${1:procedure_name}\n    @${2:param} ${3:INT}\nAS\nBEGIN\n    SET NOCOUNT ON;\n    ${cursor}\nEND".to_string(),
            description: Some("Create stored procedure".to_string()),
            category: Some("Procedure".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "ap".to_string(),
            name: "ALTER PROCEDURE".to_string(),
            content: "ALTER PROCEDURE ${cursor}".to_string(),
            description: Some("Alter stored procedure".to_string()),
            category: Some("Procedure".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "exec".to_string(),
            name: "EXEC".to_string(),
            content: "EXEC ${1:procedure_name} ${cursor}".to_string(),
            description: Some("Execute stored procedure".to_string()),
            category: Some("Procedure".to_string()),
            drivers: mssql_only(),
        },

        // Transaction snippets
        CreateSnippetInput {
            trigger: "tran".to_string(),
            name: "Transaction".to_string(),
            content: "BEGIN TRANSACTION;\nBEGIN TRY\n    ${cursor}\n    COMMIT TRANSACTION;\nEND TRY\nBEGIN CATCH\n    ROLLBACK TRANSACTION;\n    THROW;\nEND CATCH".to_string(),
            description: Some("Transaction with error handling".to_string()),
            category: Some("Transaction".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "tc".to_string(),
            name: "TRY CATCH".to_string(),
            content: "BEGIN TRY\n    ${cursor}\nEND TRY\nBEGIN CATCH\n    SELECT ERROR_MESSAGE() AS ErrorMessage;\nEND CATCH".to_string(),
            description: Some("Try-Catch block".to_string()),
            category: Some("Transaction".to_string()),
            drivers: mssql_only(),
        },

        // Variable snippets
        CreateSnippetInput {
            trigger: "decl".to_string(),
            name: "DECLARE".to_string(),
            content: "DECLARE @${1:variable} ${2:INT} = ${cursor}".to_string(),
            description: Some("Declare variable".to_string()),
            category: Some("Variable".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "dect".to_string(),
            name: "DECLARE TABLE".to_string(),
            content: "DECLARE @${1:table} TABLE (\n    ${2:id} INT,\n    ${cursor}\n)".to_string(),
            description: Some("Declare table variable".to_string()),
            category: Some("Variable".to_string()),
            drivers: mssql_only(),
        },

        // Utility snippets
        CreateSnippetInput {
            trigger: "iff".to_string(),
            name: "IF EXISTS".to_string(),
            content: "IF EXISTS (SELECT 1 FROM ${1:table} WHERE ${2:condition})\nBEGIN\n    ${cursor}\nEND".to_string(),
            description: Some("If exists check".to_string()),
            category: Some("Control Flow".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "ifn".to_string(),
            name: "IF NOT EXISTS".to_string(),
            content: "IF NOT EXISTS (SELECT 1 FROM ${1:table} WHERE ${2:condition})\nBEGIN\n    ${cursor}\nEND".to_string(),
            description: Some("If not exists check".to_string()),
            category: Some("Control Flow".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "wh".to_string(),
            name: "WHILE".to_string(),
            content: "WHILE ${1:condition}\nBEGIN\n    ${cursor}\nEND".to_string(),
            description: Some("While loop".to_string()),
            category: Some("Control Flow".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "case".to_string(),
            name: "CASE".to_string(),
            content: "CASE\n    WHEN ${1:condition} THEN ${2:result}\n    ELSE ${3:default}\nEND".to_string(),
            description: Some("CASE expression".to_string()),
            category: Some("Expression".to_string()),
            drivers: None,
        },

        // Info snippets
        CreateSnippetInput {
            trigger: "cols".to_string(),
            name: "Column Info".to_string(),
            content: "SELECT c.name AS ColumnName, t.name AS DataType, c.max_length, c.is_nullable\nFROM sys.columns c\nINNER JOIN sys.types t ON c.user_type_id = t.user_type_id\nWHERE c.object_id = OBJECT_ID('${cursor}')\nORDER BY c.column_id".to_string(),
            description: Some("Get column info for table".to_string()),
            category: Some("Info".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "tbls".to_string(),
            name: "List Tables".to_string(),
            content: "SELECT TABLE_SCHEMA, TABLE_NAME\nFROM INFORMATION_SCHEMA.TABLES\nWHERE TABLE_TYPE = 'BASE TABLE'\nORDER BY TABLE_SCHEMA, TABLE_NAME".to_string(),
            description: Some("List all tables".to_string()),
            category: Some("Info".to_string()),
            drivers: mssql_only(),
        },
        CreateSnippetInput {
            trigger: "idxs".to_string(),
            name: "Index Info".to_string(),
            content: "SELECT i.name AS IndexName, i.type_desc, c.name AS ColumnName\nFROM sys.indexes i\nINNER JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id\nINNER JOIN sys.columns c ON ic.object_id = c.object_id AND ic.column_id = c.column_id\nWHERE i.object_id = OBJECT_ID('${cursor}')\nORDER BY i.name, ic.key_ordinal".to_string(),
            description: Some("Get index info for table".to_string()),
            category: Some("Info".to_string()),
            drivers: mssql_only(),
        },

        // PostgreSQL snippets
        CreateSnippetInput {
            trigger: "selt".to_string(),
            name: "SELECT LIMIT".to_string(),
            content: "SELECT * FROM ${cursor}\nLIMIT ${1:100}".to_string(),
            description: Some("Select first N rows".to_string()),
            category: Some("Select".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "updj".to_string(),
            name: "UPDATE FROM".to_string(),
            content: "UPDATE ${1:table1} t1\nSET ${2:column} = t2.${3:column}\nFROM ${4:table2} t2\nWHERE t1.${5:id} = t2.${6:id}${cursor}".to_string(),
            description: Some("Update from another table".to_string()),
            category: Some("Update".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "upsert".to_string(),
            name: "INSERT ON CONFLICT".to_string(),
            content: "INSERT INTO ${1:table} (${2:columns})\nVALUES (${3:values})\nON CONFLICT (${4:key}) DO UPDATE\nSET ${5:column} = EXCLUDED.${5:column}${cursor}".to_string(),
            description: Some("Insert or update on key conflict".to_string()),
            category: Some("Insert".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "insr".to_string(),
            name: "INSERT RETURNING".to_string(),
            content: "INSERT INTO ${1:table} (${2:columns})\nVALUES (${3:values})\nRETURNING ${cursor}".to_string(),
            description: Some("Insert and return the new rows".to_string()),
            category: Some("Insert".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "di".to_string(),
            name: "DROP INDEX".to_string(),
            content: "DROP INDEX IF EXISTS ${cursor}".to_string(),
            description: Some("Drop index".to_string()),
            category: Some("DDL".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "ct".to_string(),
            name: "CREATE TABLE".to_string(),
            content: "CREATE TABLE ${1:table_name} (\n    ${2:id} BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,\n    ${cursor}\n)".to_string(),
            description: Some("Create new table".to_string()),
            category: Some("DDL".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "cte".to_string(),
            name: "CTE".to_string(),
            content: "WITH ${1:cte_name} AS (\n    ${2:SELECT * FROM table}\n)\nSELECT * FROM ${1:cte_name}${cursor}".to_string(),
            description: Some("Common Table Expression".to_string()),
            category: Some("CTE".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "rcte".to_string(),
            name: "Recursive CTE".to_string(),
            content: "WITH RECURSIVE ${1:cte_name} AS (\n    -- Anchor\n    SELECT ${2:columns}\n    FROM ${3:table}\n    WHERE ${4:condition}\n\n    UNION ALL\n\n    -- Recursive\n    SELECT ${2:columns}\n    FROM ${3:table} t\n    INNER JOIN ${1:cte_name} c ON t.${5:parent_id} = c.${6:id}\n)\nSELECT * FROM ${1:cte_name}${cursor}".to_string(),
            description: Some("Recursive Common Table Expression".to_string()),
            category: Some("CTE".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "cf".to_string(),
            name: "CREATE FUNCTION".to_string(),
            content: "CREATE OR REPLACE FUNCTION ${1:function_name}(${2:arg} ${3:integer})\nRETURNS ${4:integer}\nLANGUAGE plpgsql\nAS $$\nBEGIN\n    ${cursor}\nEND;\n$$;".to_string(),
            description: Some("Create PL/pgSQL function".to_string()),
            category: Some("Procedure".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "cp".to_string(),
            name: "CREATE PROCEDURE".to_string(),
            content: "CREATE OR REPLACE PROCEDURE ${1:procedure_name}(${2:arg} ${3:integer})\nLANGUAGE plpgsql\nAS $$\nBEGIN\n    ${cursor}\nEND;\n$$;".to_string(),
            description: Some("Create PL/pgSQL procedure".to_string()),
            category: Some("Procedure".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "call".to_string(),
            name: "CALL".to_string(),
            content: "CALL ${1:procedure_name}(${cursor});".to_string(),
            description: Some("Call a procedure".to_string()),
            category: Some("Procedure".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "do".to_string(),
            name: "DO block".to_string(),
            content: "DO $$\nBEGIN\n    ${cursor}\nEND;\n$$;".to_string(),
            description: Some("Anonymous PL/pgSQL block".to_string()),
            category: Some("Control Flow".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "tran".to_string(),
            name: "Transaction".to_string(),
            content: "BEGIN;\n    ${cursor}\nCOMMIT;".to_string(),
            description: Some("Transaction block".to_string()),
            category: Some("Transaction".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "expl".to_string(),
            name: "EXPLAIN ANALYZE".to_string(),
            content: "EXPLAIN (ANALYZE, BUFFERS)\n${cursor}".to_string(),
            description: Some("Execution plan with actual timings".to_string()),
            category: Some("Info".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "cols".to_string(),
            name: "Column Info".to_string(),
            content: "SELECT column_name, data_type, character_maximum_length, is_nullable, column_default\nFROM information_schema.columns\nWHERE table_schema = '${1:public}' AND table_name = '${cursor}'\nORDER BY ordinal_position".to_string(),
            description: Some("Get column info for table".to_string()),
            category: Some("Info".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "tbls".to_string(),
            name: "List Tables".to_string(),
            content: "SELECT table_schema, table_name\nFROM information_schema.tables\nWHERE table_type = 'BASE TABLE'\n  AND table_schema NOT IN ('pg_catalog', 'information_schema')\nORDER BY table_schema, table_name".to_string(),
            description: Some("List all tables".to_string()),
            category: Some("Info".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "idxs".to_string(),
            name: "Index Info".to_string(),
            content: "SELECT indexname, indexdef\nFROM pg_indexes\nWHERE schemaname = '${1:public}' AND tablename = '${cursor}'\nORDER BY indexname".to_string(),
            description: Some("Get index info for table".to_string()),
            category: Some("Info".to_string()),
            drivers: postgres_only(),
        },
        CreateSnippetInput {
            trigger: "act".to_string(),
            name: "Active Queries".to_string(),
            content: "SELECT pid, usename, state, now() - query_start AS duration, query\nFROM pg_stat_activity\nWHERE state <> 'idle' AND pid <> pg_backend_pid()\nORDER BY duration DESC".to_string(),
            description: Some("Running queries on the server".to_string()),
            category: Some("Info".to_string()),
            drivers: postgres_only(),
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
                    drivers TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                -- Index for quick trigger lookups
                CREATE INDEX IF NOT EXISTS idx_snippets_trigger ON snippets(trigger);
                CREATE INDEX IF NOT EXISTS idx_snippets_enabled ON snippets(enabled);
                "#,
            )?;
            Ok(())
        })?;
        self.migrate_snippet_drivers()
    }

    /// Add the `drivers` column to databases created before snippets were
    /// scoped per database. Every built-in up to then was written for
    /// SQL Server, so built-ins that are now SQL Server-only get tagged —
    /// otherwise they'd keep showing up in PostgreSQL editors and clash with
    /// the PostgreSQL built-in seeded under the same trigger.
    fn migrate_snippet_drivers(&self) -> StorageResult<()> {
        self.with_connection(|conn| {
            let has_column: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('snippets') WHERE name = 'drivers'",
                [],
                |row| row.get(0),
            )?;
            if has_column {
                return Ok(());
            }
            conn.execute("ALTER TABLE snippets ADD COLUMN drivers TEXT", [])?;

            let builtins = get_builtin_snippets();
            let mssql = drivers_column(mssql_only().as_deref());
            for builtin in &builtins {
                let mssql_scoped = drivers_column(builtin.drivers.as_deref()) == mssql;
                let has_generic = builtins
                    .iter()
                    .any(|b| b.trigger == builtin.trigger && b.drivers.is_none());
                if mssql_scoped && !has_generic {
                    conn.execute(
                        "UPDATE snippets SET drivers = ?1 WHERE is_builtin = 1 AND drivers IS NULL AND trigger = ?2",
                        params![mssql, builtin.trigger],
                    )?;
                }
            }
            Ok(())
        })
    }

    /// Seed built-in snippets if they don't exist
    pub fn seed_builtin_snippets(&self) -> StorageResult<usize> {
        let builtins = get_builtin_snippets();
        let mut count = 0;

        for snippet in builtins {
            // A built-in is identified by its trigger within its driver scope
            let drivers = drivers_column(snippet.drivers.as_deref());
            let exists: bool = self.with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) > 0 FROM snippets WHERE trigger = ?1 AND drivers IS ?2 AND is_builtin = 1",
                    params![&snippet.trigger, &drivers],
                    |row| row.get(0),
                )
            })?;

            if !exists {
                self.create_snippet_internal(&snippet, true)?;
                count += 1;
            }
        }

        Ok(count)
    }

    /// Create a new snippet (internal - with is_builtin flag)
    fn create_snippet_internal(
        &self,
        input: &CreateSnippetInput,
        is_builtin: bool,
    ) -> StorageResult<Snippet> {
        let id = Uuid::new_v4().to_string();
        let drivers = drivers_column(input.drivers.as_deref());

        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO snippets (id, trigger, name, content, description, is_builtin, enabled, category, drivers)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)
                "#,
                params![
                    id,
                    input.trigger,
                    input.name,
                    input.content,
                    input.description,
                    is_builtin,
                    input.category,
                    drivers
                ],
            )?;

            conn.query_row(
                &format!("SELECT {SNIPPET_COLUMNS} FROM snippets WHERE id = ?1"),
                params![id],
                row_to_snippet,
            )
        })
    }

    /// Create a new user-defined snippet
    pub fn create_snippet(&self, input: CreateSnippetInput) -> StorageResult<Snippet> {
        self.create_snippet_internal(&input, false) // is_builtin = false for user-defined
    }

    /// Get all snippets
    pub fn get_all_snippets(&self) -> StorageResult<Vec<Snippet>> {
        self.query_snippets("ORDER BY category NULLS LAST, trigger")
    }

    /// Get only enabled snippets
    pub fn get_enabled_snippets(&self) -> StorageResult<Vec<Snippet>> {
        self.query_snippets("WHERE enabled = 1 ORDER BY category NULLS LAST, trigger")
    }

    fn query_snippets(&self, clause: &str) -> StorageResult<Vec<Snippet>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(&format!("SELECT {SNIPPET_COLUMNS} FROM snippets {clause}"))?;
            let snippets = stmt
                .query_map([], row_to_snippet)?
                .filter_map(|r| r.ok())
                .collect();
            Ok(snippets)
        })
    }

    /// Get a snippet by ID
    pub fn get_snippet(&self, id: &str) -> StorageResult<Option<Snippet>> {
        self.with_connection(|conn| {
            let result = conn.query_row(
                &format!("SELECT {SNIPPET_COLUMNS} FROM snippets WHERE id = ?1"),
                params![id],
                row_to_snippet,
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
                &format!("SELECT {SNIPPET_COLUMNS} FROM snippets WHERE trigger = ?1 AND enabled = 1"),
                params![trigger],
                row_to_snippet,
            );

            match result {
                Ok(snippet) => Ok(Some(snippet)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// Update an existing snippet
    pub fn update_snippet(
        &self,
        id: &str,
        input: UpdateSnippetInput,
    ) -> StorageResult<Option<Snippet>> {
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
            if let Some(ref drivers) = input.drivers {
                updates.push("drivers = ?");
                values.push(Box::new(drivers_column(Some(drivers))));
            }

            if updates.is_empty() {
                // existing is Some here (checked above)
                return Ok(existing);
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
                &format!("SELECT {SNIPPET_COLUMNS} FROM snippets WHERE id = ?1"),
                params![id],
                |row| row_to_snippet(row).map(Some),
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

            // Find the original builtin: same trigger, same driver scope
            let scope = drivers_column(snippet.drivers.as_deref());
            let builtins = get_builtin_snippets();
            if let Some(original) = builtins.iter().find(|s| {
                s.trigger == snippet.trigger && drivers_column(s.drivers.as_deref()) == scope
            }) {
                return self.update_snippet(
                    id,
                    UpdateSnippetInput {
                        trigger: None,
                        name: Some(original.name.clone()),
                        content: Some(original.content.clone()),
                        description: original.description.clone(),
                        category: original.category.clone(),
                        enabled: Some(true),
                        drivers: None,
                    },
                );
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn create_test_db() -> (DatabaseManager, PathBuf) {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!(
            "larik_snippet_test_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&db_path);
        let manager = DatabaseManager::new(db_path.clone()).unwrap();
        manager.init_snippets_schema().unwrap();
        (manager, db_path)
    }

    #[test]
    fn test_create_and_get_snippet() {
        let (manager, db_path) = create_test_db();

        let snippet = manager
            .create_snippet(CreateSnippetInput {
                trigger: "test".to_string(),
                name: "Test Snippet".to_string(),
                content: "SELECT * FROM ${cursor}".to_string(),
                description: Some("A test snippet".to_string()),
                category: Some("Test".to_string()),
                drivers: None,
            })
            .unwrap();

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

        let snippet = manager
            .create_snippet(CreateSnippetInput {
                trigger: "usrsnp".to_string(),
                name: "User Snippet".to_string(),
                content: "test".to_string(),
                description: None,
                category: None,
                drivers: None,
            })
            .unwrap();

        let deleted = manager.delete_snippet(&snippet.id).unwrap();
        assert!(deleted);

        let fetched = manager.get_snippet(&snippet.id).unwrap();
        assert!(fetched.is_none());

        let _ = std::fs::remove_file(&db_path);
    }

    fn builtin(manager: &DatabaseManager, trigger: &str, driver: Option<&str>) -> Snippet {
        manager
            .get_all_snippets()
            .unwrap()
            .into_iter()
            .find(|s| {
                s.is_builtin
                    && s.trigger == trigger
                    && s.drivers.as_ref().map(|d| d.join(",")).as_deref() == driver
            })
            .unwrap_or_else(|| panic!("no builtin {trigger} for {driver:?}"))
    }

    #[test]
    fn test_builtins_are_scoped_per_driver() {
        let (manager, db_path) = create_test_db();
        manager.seed_builtin_snippets().unwrap();

        // Same trigger, one body per database
        assert!(builtin(&manager, "selt", Some("mssql")).content.contains("TOP"));
        assert!(builtin(&manager, "selt", Some("postgres")).content.contains("LIMIT"));
        // Generic snippets apply everywhere
        assert!(builtin(&manager, "sel", None).drivers.is_none());
        // T-SQL-only snippets have no PostgreSQL twin
        assert_eq!(builtin(&manager, "decl", Some("mssql")).trigger, "decl");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_migration_tags_legacy_tsql_builtins() {
        let (manager, db_path) = create_test_db();
        // Simulate a database from before driver scoping: no drivers column,
        // T-SQL built-ins stored unscoped.
        manager
            .with_connection(|conn| {
                conn.execute_batch(
                    "DROP TABLE snippets;
                     CREATE TABLE snippets (
                         id TEXT PRIMARY KEY, trigger TEXT NOT NULL, name TEXT NOT NULL,
                         content TEXT NOT NULL, description TEXT,
                         is_builtin INTEGER NOT NULL DEFAULT 0, enabled INTEGER NOT NULL DEFAULT 1,
                         category TEXT,
                         created_at TEXT NOT NULL DEFAULT (datetime('now')),
                         updated_at TEXT NOT NULL DEFAULT (datetime('now')));
                     INSERT INTO snippets (id, trigger, name, content, is_builtin)
                     VALUES ('a', 'selt', 'SELECT TOP', 'SELECT TOP 10 * FROM t', 1),
                            ('b', 'sel', 'SELECT *', 'SELECT * FROM t', 1),
                            ('c', 'mine', 'Mine', 'SELECT 1', 0);",
                )
            })
            .unwrap();

        manager.init_snippets_schema().unwrap();
        manager.seed_builtin_snippets().unwrap();

        let by_id = |id: &str| manager.get_snippet(id).unwrap().unwrap();
        assert_eq!(by_id("a").drivers, Some(vec!["mssql".to_string()]));
        assert_eq!(by_id("b").drivers, None);
        assert_eq!(by_id("c").drivers, None); // user snippets stay unscoped
        // The legacy row is the SQL Server builtin; PostgreSQL got its own
        assert_eq!(builtin(&manager, "selt", Some("mssql")).id, "a");
        assert!(builtin(&manager, "selt", Some("postgres")).content.contains("LIMIT"));
        // No duplicate generic builtin was seeded
        let sel_count = manager
            .get_all_snippets()
            .unwrap()
            .iter()
            .filter(|s| s.trigger == "sel" && s.is_builtin)
            .count();
        assert_eq!(sel_count, 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_reset_builtin_respects_driver_scope() {
        let (manager, db_path) = create_test_db();
        manager.seed_builtin_snippets().unwrap();

        let pg = builtin(&manager, "selt", Some("postgres"));
        manager
            .update_snippet(
                &pg.id,
                UpdateSnippetInput {
                    content: Some("edited".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        let reset = manager.reset_builtin_snippet(&pg.id).unwrap().unwrap();
        assert!(reset.content.contains("LIMIT"));
        assert_eq!(reset.drivers, Some(vec!["postgres".to_string()]));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_user_snippet_driver_scope() {
        let (manager, db_path) = create_test_db();
        let snippet = manager
            .create_snippet(CreateSnippetInput {
                trigger: "pgx".to_string(),
                name: "PG".to_string(),
                content: "SELECT 1".to_string(),
                description: None,
                category: None,
                // Normalized: aliases resolved, mongodb/unknown dropped, sorted
                drivers: Some(vec!["pg".into(), "mongodb".into(), "oracle".into(), "mssql".into()]),
            })
            .unwrap();
        assert_eq!(
            snippet.drivers,
            Some(vec!["mssql".to_string(), "postgres".to_string()])
        );

        // An empty scope means every SQL database again
        let updated = manager
            .update_snippet(
                &snippet.id,
                UpdateSnippetInput {
                    drivers: Some(Vec::new()),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(updated.drivers, None);

        let _ = std::fs::remove_file(&db_path);
    }
}
