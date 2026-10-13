// Schema Metadata Fetching
// Queries PostgreSQL catalogs (pg_catalog) for database schema information,
// mirroring the output shapes of `db/mssql/schema.rs`. All queries are
// prepared with parameters — user input (schema filter, table names) never
// enters the SQL text.

use super::connection::{connect_dedicated, statement_error};
use crate::db::config::ConnectionConfig;
use crate::db::error::ConnectionError;
use crate::db::schema::{ColumnInfo, RelationshipInfo, RoutineInfo, SchemaInfo, TableInfo};
use tokio_postgres::Client;

/// Schemas owned by PostgreSQL itself; hidden from the explorer tree.
const SYSTEM_SCHEMA_CONDITION: &str =
    "n.nspname NOT LIKE 'pg\\_%' AND n.nspname NOT LIKE 'pg\\_temp%' AND n.nspname <> 'information_schema'";

/// Open a dedicated connection to `database` (per-database, like the SQL
/// Server driver's `USE` handling — a postgres client cannot switch
/// databases after connect).
async fn connect_to_database(
    config: &ConnectionConfig,
    database: &str,
) -> Result<Client, ConnectionError> {
    connect_dedicated(config, Some(database)).await
}

/// Fetch schema information for a database.
pub(super) async fn fetch_schema(
    config: &ConnectionConfig,
    database: &str,
    schema_filter: Option<&str>,
) -> Result<SchemaInfo, ConnectionError> {
    let conn = connect_to_database(config, database).await?;

    let schemas = fetch_schemas(&conn).await?;
    let tables = fetch_tables_and_views(&conn, schema_filter).await?;
    let relationships = fetch_relationships(&conn, schema_filter).await?;
    let routines = fetch_routines(&conn, schema_filter).await?;

    Ok(SchemaInfo {
        database_name: database.to_string(),
        schemas,
        tables,
        relationships,
        routines,
        fetched_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Fetch all user schema names.
async fn fetch_schemas(conn: &Client) -> Result<Vec<String>, ConnectionError> {
    let sql = format!(
        "SELECT n.nspname FROM pg_namespace n WHERE {SYSTEM_SCHEMA_CONDITION} ORDER BY n.nspname"
    );
    let rows = conn.query(&sql, &[]).await.map_err(statement_error)?;
    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

/// Fetch tables and views (incl. partitioned/materialized) with their columns.
async fn fetch_tables_and_views(
    conn: &Client,
    schema_filter: Option<&str>,
) -> Result<Vec<TableInfo>, ConnectionError> {
    let sql = format!(
        r#"
            SELECT
                n.nspname,
                c.relname,
                CASE WHEN c.relkind IN ('r', 'p', 'f') THEN 'BASE TABLE' ELSE 'VIEW' END,
                a.attname,
                format_type(a.atttypid, a.atttypmod),
                NOT a.attnotnull,
                a.attnum::int,
                EXISTS (
                    SELECT 1 FROM pg_index i
                    WHERE i.indrelid = c.oid AND i.indisprimary AND a.attnum = ANY(i.indkey)
                ),
                a.attidentity <> ''::"char",
                a.attgenerated <> ''::"char",
                pg_get_expr(d.adbin, d.adrelid)
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
            LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
            WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
              AND {SYSTEM_SCHEMA_CONDITION}
              AND ($1::text IS NULL OR n.nspname = $1)
            ORDER BY n.nspname, c.relname, a.attnum
        "#
    );
    let rows = conn
        .query(&sql, &[&schema_filter])
        .await
        .map_err(statement_error)?;

    let mut tables: Vec<TableInfo> = Vec::new();
    let mut last_key: Option<(String, String)> = None;
    for row in &rows {
        let schema_name: String = row.get(0);
        let table_name: String = row.get(1);
        let key = (schema_name.clone(), table_name.clone());
        if last_key.as_ref() != Some(&key) {
            tables.push(TableInfo {
                schema_name,
                table_name,
                table_type: row.get(2),
                columns: Vec::new(),
            });
            last_key = Some(key);
        }
        let Some(current) = tables.last_mut() else {
            continue;
        };
        current.columns.push(ColumnInfo {
            name: row.get(3),
            data_type: row.get(4),
            max_length: None,
            precision: None,
            scale: None,
            is_nullable: row.get(5),
            is_primary_key: row.get(7),
            is_identity: row.get(8),
            is_computed: row.get(9),
            column_default: row.get(10),
            ordinal_position: row.get(6),
        });
    }
    Ok(tables)
}

/// Fetch foreign key relationships (used for join suggestions and the
/// reference preview).
async fn fetch_relationships(
    conn: &Client,
    schema_filter: Option<&str>,
) -> Result<Vec<RelationshipInfo>, ConnectionError> {
    let sql = r#"
            SELECT
                con.conname,
                k.ord::int,
                src_ns.nspname, src_tbl.relname, src_col.attname,
                tgt_ns.nspname, tgt_tbl.relname, tgt_col.attname
            FROM pg_constraint con
            JOIN pg_class src_tbl ON src_tbl.oid = con.conrelid
            JOIN pg_namespace src_ns ON src_ns.oid = src_tbl.relnamespace
            JOIN pg_class tgt_tbl ON tgt_tbl.oid = con.confrelid
            JOIN pg_namespace tgt_ns ON tgt_ns.oid = tgt_tbl.relnamespace
            CROSS JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord)
            JOIN pg_attribute src_col ON src_col.attrelid = src_tbl.oid AND src_col.attnum = k.attnum
            JOIN pg_attribute tgt_col ON tgt_col.attrelid = tgt_tbl.oid AND tgt_col.attnum = con.confkey[k.ord]
            WHERE con.contype = 'f'
              AND ($1::text IS NULL OR src_ns.nspname = $1 OR tgt_ns.nspname = $1)
            ORDER BY con.conname, k.ord
        "#;
    let rows = conn
        .query(sql, &[&schema_filter])
        .await
        .map_err(statement_error)?;

    Ok(rows
        .iter()
        .map(|r| RelationshipInfo {
            constraint_name: r.get(0),
            ordinal_position: r.get(1),
            source_schema_name: r.get(2),
            source_table_name: r.get(3),
            source_column_name: r.get(4),
            target_schema_name: r.get(5),
            target_table_name: r.get(6),
            target_column_name: r.get(7),
        })
        .collect())
}

/// Fetch functions and procedures with their parameters.
///
/// - One entry per overload (keyed by `pg_proc.oid`), so `f(int)` and
///   `f(int, text)` don't merge their parameter lists.
/// - Parameters come from `proallargtypes` (all args, incl. OUT/TABLE) when
///   present, falling back to `proargtypes` (input args only) — the arrays
///   `proargmodes`/`proargnames` are indexed against the former.
/// - `LEFT JOIN LATERAL` keeps routines with no parameters.
///
/// Parameter defaults are not resolved (pg_node_tree parsing is brittle);
/// `has_default` is always `false` — display-only metadata.
async fn fetch_routines(
    conn: &Client,
    schema_filter: Option<&str>,
) -> Result<Vec<RoutineInfo>, ConnectionError> {
    let sql = format!(
        r#"
            SELECT
                p.oid::bigint,
                n.nspname,
                p.proname,
                CASE WHEN p.prokind = 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END,
                CASE WHEN p.prokind = 'p' THEN NULL ELSE pg_get_function_result(p.oid) END,
                CASE m.mode
                    WHEN 'o' THEN 'OUT' WHEN 't' THEN 'OUT' WHEN 'b' THEN 'INOUT'
                    WHEN 'v' THEN 'VARIADIC' ELSE 'IN'
                END,
                COALESCE(m.name, ''),
                format_type(m.type_oid, NULL),
                m.ord::int
            FROM pg_proc p
            JOIN pg_namespace n ON n.oid = p.pronamespace
            LEFT JOIN LATERAL (
                SELECT
                    a.ord,
                    a.type_oid,
                    p.proargmodes[a.ord] AS mode,
                    p.proargnames[a.ord] AS name
                FROM unnest(COALESCE(p.proallargtypes, p.proargtypes::oid[]))
                    WITH ORDINALITY AS a(type_oid, ord)
            ) m ON true
            WHERE {SYSTEM_SCHEMA_CONDITION}
              AND p.prokind IN ('f', 'p')
              AND ($1::text IS NULL OR n.nspname = $1)
            ORDER BY n.nspname, p.proname, p.oid, m.ord
        "#
    );
    let rows = conn
        .query(&sql, &[&schema_filter])
        .await
        .map_err(statement_error)?;

    let mut routines: Vec<RoutineInfo> = Vec::new();
    let mut last_oid: Option<i64> = None;
    for row in &rows {
        let oid: i64 = row.get(0);
        if last_oid != Some(oid) {
            routines.push(RoutineInfo {
                schema_name: row.get(1),
                routine_name: row.get(2),
                routine_type: row.get(3),
                return_type: row.get(4),
                parameters: Vec::new(),
            });
            last_oid = Some(oid);
        }
        // No parameter row for a zero-argument routine (LEFT JOIN miss).
        let Some(ordinal_position) = row.get::<_, Option<i32>>(8) else {
            continue;
        };
        let Some(current) = routines.last_mut() else {
            continue;
        };
        current.parameters.push(crate::db::schema::ParameterInfo {
            name: row.get(6),
            data_type: row.get::<_, Option<String>>(7).unwrap_or_default(),
            max_length: None,
            precision: None,
            scale: None,
            parameter_mode: row.get(5),
            ordinal_position,
            has_default: false,
        });
    }
    Ok(routines)
}

/// Get columns for a specific table or view.
pub(super) async fn fetch_table_columns(
    config: &ConnectionConfig,
    database: &str,
    schema_name: &str,
    table_name: &str,
) -> Result<Vec<ColumnInfo>, ConnectionError> {
    let conn = connect_to_database(config, database).await?;
    let sql = r#"
            SELECT
                a.attname,
                format_type(a.atttypid, a.atttypmod),
                NOT a.attnotnull,
                a.attnum::int,
                EXISTS (
                    SELECT 1 FROM pg_index i
                    WHERE i.indrelid = c.oid AND i.indisprimary AND a.attnum = ANY(i.indkey)
                ),
                a.attidentity <> ''::"char",
                a.attgenerated <> ''::"char",
                pg_get_expr(d.adbin, d.adrelid)
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
            LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
            WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
              AND n.nspname = $1 AND c.relname = $2
            ORDER BY a.attnum
        "#;
    let rows = conn
        .query(sql, &[&schema_name, &table_name])
        .await
        .map_err(statement_error)?;

    Ok(rows
        .iter()
        .map(|r| ColumnInfo {
            name: r.get(0),
            data_type: r.get(1),
            max_length: None,
            precision: None,
            scale: None,
            is_nullable: r.get(2),
            is_primary_key: r.get(4),
            is_identity: r.get(5),
            is_computed: r.get(6),
            column_default: r.get(7),
            ordinal_position: r.get(3),
        })
        .collect())
}
