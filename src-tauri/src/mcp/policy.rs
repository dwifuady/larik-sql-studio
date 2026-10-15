// Access policy for the embedded MCP server.
//
// Fail-closed rules:
// - A space is visible to MCP only when the user set `mcp_access` to `read`
//   or `write`; unknown stored values fall back to `off`.
// - At access level `read`, statements are gated per engine:
//   - PostgreSQL: engine-level enforcement (`default_transaction_read_only` via
//     startup options on the dedicated session), plus a gate that keeps the
//     session from switching it off (one statement per unit, no session or
//     transaction control).
//   - MongoDB: allow-list of read methods on the parsed shell statement, with
//     write pipeline stages (`$out`, `$merge`) denied.
//   - MS-SQL: no session-level read-only exists in T-SQL, so a conservative
//     classifier denies anything not provably a read. For a hard guarantee use
//     a read-only login; the Settings UI says the same.

use crate::db::mongodb::shell::{parse_single, MethodCall, Statement, Value};

/// Exposure level of a space to the MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Off,
    Read,
    Write,
}

impl Access {
    pub fn parse(value: &str) -> Self {
        match value {
            "read" => Self::Read,
            "write" => Self::Write,
            _ => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Read => "read",
            Self::Write => "write",
        }
    }

    pub fn is_off(self) -> bool {
        matches!(self, Self::Off)
    }

    pub fn is_read(self) -> bool {
        matches!(self, Self::Read)
    }
}

/// MongoDB methods allowed at access level `read`.
///
/// The driver also implements `insertOne`/`updateMany`/`replaceOne`/
/// `deleteOne`/`deleteMany` and the arbitrary `runCommand`/`adminCommand` —
/// all write-capable and therefore denied here.
const MONGO_READ_METHODS: &[&str] = &[
    "find",
    "findOne",
    "aggregate",
    "count",
    "countDocuments",
    "estimatedDocumentCount",
    "distinct",
    "getIndexes",
    "getCollectionNames",
    "stats",
];

/// Classify one already-split mongo statement at access level `read`.
///
/// `Some(false)` = denied. `Some(true)` = allowed. `None` = undecidable: either
/// the SQL `SELECT` subset (read-only by construction — it compiles to an
/// aggregation pipeline) or unparseable text the driver will reject itself.
pub fn mongo_read_allowed(unit: &str) -> Option<bool> {
    match parse_single(unit) {
        Ok(Statement::Use(_)) => Some(true),
        Ok(Statement::Show(_)) => Some(true),
        Ok(Statement::DatabaseCall { call, .. }) => Some(mongo_call_is_read(&call, &[])),
        Ok(Statement::CollectionCall { call, chain, .. }) => {
            Some(mongo_call_is_read(&call, &chain))
        }
        Err(_) => None,
    }
}

/// Pipeline stages that write: `aggregate([..., {$out: "c"}])` replaces a
/// collection and `$merge` upserts into one, so a read method carrying them
/// is not a read.
const MONGO_WRITE_STAGES: &[&str] = &["$out", "$merge"];

fn mongo_call_is_read(call: &MethodCall, chain: &[MethodCall]) -> bool {
    MONGO_READ_METHODS.contains(&call.name.as_str())
        && !std::iter::once(call)
            .chain(chain)
            .flat_map(|c| &c.args)
            .any(mongo_value_has_write_stage)
}

fn mongo_value_has_write_stage(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, v)| {
            MONGO_WRITE_STAGES.contains(&key.as_str()) || mongo_value_has_write_stage(v)
        }),
        Value::Array(items) | Value::Call { args: items, .. } => {
            items.iter().any(mongo_value_has_write_stage)
        }
        _ => false,
    }
}

/// Verbs that may lead a read statement (per semicolon-separated part).
const MSSQL_READ_LEADS: &[&str] = &["SELECT", "WITH", "DECLARE"];

/// Tokens that make a T-SQL statement non-read. Word-exact matches plus
/// `xp_`/`sp_` prefixes. `INTO` catches `SELECT ... INTO`; `OUTPUT` and `SET`
/// fail closed on session/state manipulation; `WAITFOR` kills time-based
/// blind-injection probes.
const MSSQL_BANNED: &[&str] = &[
    "INSERT",
    "UPDATE",
    "DELETE",
    "MERGE",
    "TRUNCATE",
    "CREATE",
    "ALTER",
    "DROP",
    "EXEC",
    "EXECUTE",
    "GRANT",
    "REVOKE",
    "DENY",
    "BACKUP",
    "RESTORE",
    "BULK",
    "DBCC",
    "CHECKPOINT",
    "KILL",
    "SHUTDOWN",
    "SET",
    "USE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "SAVE",
    "WAITFOR",
    "RAISERROR",
    "THROW",
    "PRINT",
    "OPENROWSET",
    "OPENQUERY",
    "OPENXML",
    "OPENDATASOURCE",
    "INTO",
    "IDENTITY",
    "OUTPUT",
];

/// Classify one already-split T-SQL unit at access level `read`.
///
/// Literals, comments and quoted identifiers are stripped first, then every
/// remaining semicolon-separated part must start with a read verb and contain
/// no banned token. Stripping before splitting matters: a `;` inside a string
/// must not desync the scanner, or `SELECT 'x;' DELETE FROM t --'` would hide
/// the DELETE inside a fake string. Anything the classifier cannot prove a
/// read is denied. Best-effort by design; for a hard guarantee use a
/// read-only login.
pub fn mssql_read_allowed(unit: &str) -> bool {
    let stripped = strip_literals(unit);
    stripped.split(';').all(|part| {
        let upper = part.to_ascii_uppercase();
        let tokens: Vec<&str> = upper
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|t| !t.is_empty())
            .collect();

        match tokens.first() {
            None => true, // comment-only fragment
            Some(lead) => {
                MSSQL_READ_LEADS.contains(lead)
                    && !tokens.iter().any(|t| {
                        MSSQL_BANNED.contains(t) || t.starts_with("XP_") || t.starts_with("SP_")
                    })
            }
        }
    })
}

/// Statement leads that control the session or transaction. With these an
/// agent could leave the read-only mode itself (`SET default_transaction_read_only
/// = off`, `BEGIN READ WRITE`, `COMMIT` then write) or reach outside the
/// database (`COPY ... TO PROGRAM`, `DO`/`CALL` bodies).
const PG_BANNED_LEADS: &[&str] = &[
    "SET",
    "RESET",
    "BEGIN",
    "START",
    "COMMIT",
    "END",
    "ROLLBACK",
    "ABORT",
    "SAVEPOINT",
    "RELEASE",
    "PREPARE",
    "EXECUTE",
    "DEALLOCATE",
    "DISCARD",
    "LOAD",
    "COPY",
    "DO",
    "CALL",
    "LISTEN",
    "UNLISTEN",
    "NOTIFY",
    "LOCK",
];

/// Words banned anywhere in a statement: `SET`/`RESET` in any position, the
/// function form of `SET` (`set_config`), side-channel connections that run
/// outside the read-only session (`dblink*`), server-file and backend control,
/// and `U&` (Unicode-escape names the gate cannot read).
const PG_BANNED_ANYWHERE: &[&str] = &[
    "U&",
    "SET",
    "RESET",
    "SET_CONFIG",
    "DBLINK",
    "DBLINK_EXEC",
    "DBLINK_CONNECT",
    "DBLINK_CONNECT_U",
    "DBLINK_SEND_QUERY",
    "LO_IMPORT",
    "LO_EXPORT",
    "PG_TERMINATE_BACKEND",
    "PG_CANCEL_BACKEND",
    "PG_RELOAD_CONF",
];

/// Classify one PostgreSQL unit at access level `read`.
///
/// The engine enforces read-only through `default_transaction_read_only`,
/// but that is a session setting any statement can switch off. Two rules keep
/// the engine-level guarantee intact:
/// - One statement per unit. Each unit runs on its own fresh connection and a
///   transaction's read-only mode is fixed when it starts, so a setting
///   changed inside a statement (even via a function called from a string,
///   e.g. `query_to_xml('select set_config(...)')`) cannot reach a write.
/// - No session/transaction control or known escape hatches (word lists
///   above), as defense in depth.
///
/// Statements are split and tokenized with the driver's own scanner (dollar
/// quotes, `E''` strings, nested comments).
pub fn postgres_read_allowed(unit: &str) -> bool {
    let statements = crate::db::postgres::statement_words(unit);
    statements.len() <= 1
        && statements.iter().all(|words| match words.first() {
            None => true,
            Some(lead) => {
                !PG_BANNED_LEADS.contains(&lead.as_str())
                    && !words
                        .iter()
                        .any(|w| PG_BANNED_ANYWHERE.contains(&w.as_str()))
            }
        })
}

/// Remove string literals ('...' with '' doubling), quoted identifiers ("...",
/// [...]) and comments (-- and /* */) so token scans see only real code.
/// Keeps `;` visible (the caller splits on it anyway).
fn strip_literals(sql: &str) -> String {
    #[derive(PartialEq)]
    enum St {
        Code,
        Line,
        Block,
        Str,
        QuotedIdent,
        Bracket,
    }

    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut st = St::Code;
    let mut block_depth = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1);
        match st {
            St::Code => match c {
                '-' if next == Some(&'-') => {
                    st = St::Line;
                    out.push(' ');
                    i += 2;
                }
                '/' if next == Some(&'*') => {
                    st = St::Block;
                    block_depth = 1;
                    out.push(' ');
                    i += 2;
                }
                '\'' | '"' | '[' => {
                    st = if c == '\'' {
                        St::Str
                    } else if c == '"' {
                        St::QuotedIdent
                    } else {
                        St::Bracket
                    };
                    out.push(' ');
                    i += 1;
                }
                _ => {
                    out.push(c);
                    i += 1;
                }
            },
            St::Line => {
                if c == '\n' {
                    st = St::Code;
                    out.push('\n');
                }
                i += 1;
            }
            St::Block => {
                if c == '/' && next == Some(&'*') {
                    block_depth += 1;
                    i += 2;
                } else if c == '*' && next == Some(&'/') {
                    block_depth -= 1;
                    i += 2;
                    if block_depth == 0 {
                        st = St::Code;
                    }
                } else {
                    i += 1;
                }
            }
            // Doubled delimiters ('', "", ]]) stay inside; a single one exits.
            St::Str => {
                if c == '\'' && next != Some(&'\'') {
                    st = St::Code;
                }
                i += if c == '\'' && next == Some(&'\'') {
                    2
                } else {
                    1
                };
            }
            St::QuotedIdent => {
                if c == '"' && next != Some(&'"') {
                    st = St::Code;
                }
                i += if c == '"' && next == Some(&'"') { 2 } else { 1 };
            }
            St::Bracket => {
                if c == ']' && next != Some(&']') {
                    st = St::Code;
                }
                i += if c == ']' && next == Some(&']') { 2 } else { 1 };
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // ---- MSSQL classifier ----

    #[test]
    fn mssql_allows_plain_reads() {
        assert!(mssql_read_allowed("SELECT * FROM dbo.users"));
        assert!(mssql_read_allowed(
            "select id, name from users where id = 1"
        ));
        assert!(mssql_read_allowed(
            "WITH cte AS (SELECT 1 AS x) SELECT x FROM cte"
        ));
        assert!(mssql_read_allowed("DECLARE @x INT; SELECT @x AS v"));
        assert!(mssql_read_allowed(
            "SELECT * FROM t WHERE name = 'DELETE FROM users'"
        ));
        assert!(mssql_read_allowed("SELECT 1 -- INSERT something\nFROM t"));
        assert!(mssql_read_allowed("/* INSERT */ SELECT 1"));
        assert!(mssql_read_allowed("SELECT [update], [delete] FROM t"));
        assert!(mssql_read_allowed(
            "SELECT * FROM t ORDER BY x OFFSET 0 ROWS"
        ));
        assert!(mssql_read_allowed("SELECT created_at, deleted_at FROM t"));
    }

    #[test]
    fn mssql_denies_writes_and_exotics() {
        assert!(!mssql_read_allowed("DELETE FROM users"));
        assert!(!mssql_read_allowed("INSERT INTO t VALUES (1)"));
        assert!(!mssql_read_allowed("UPDATE t SET a = 1"));
        assert!(!mssql_read_allowed("TRUNCATE TABLE t"));
        assert!(!mssql_read_allowed("SELECT * INTO t2 FROM t1"));
        assert!(!mssql_read_allowed("WITH cte AS (SELECT 1) DELETE FROM t"));
        assert!(!mssql_read_allowed("SELECT 1; DELETE FROM t"));
        assert!(!mssql_read_allowed("EXEC dbo.usp_do_stuff"));
        assert!(!mssql_read_allowed("EXECUTE xp_cmdshell 'dir'"));
        assert!(!mssql_read_allowed("master..xp_cmdshell 'dir'"));
        assert!(!mssql_read_allowed("dbo.usp_do_stuff"));
        assert!(!mssql_read_allowed(
            "SELECT * FROM OPENROWSET('x', 'y', 'z')"
        ));
        assert!(!mssql_read_allowed("SET NOCOUNT ON; SELECT 1"));
        assert!(!mssql_read_allowed("USE msdb; SELECT 1"));
        assert!(!mssql_read_allowed("SELECT 1; WAITFOR DELAY '0:0:5'"));
        assert!(!mssql_read_allowed("BEGIN TRAN; SELECT 1"));
        assert!(!mssql_read_allowed(
            "SELECT * FROM OPENDATASOURCE('SQLNCLI', 'Data Source=x').db.dbo.t"
        ));
    }

    #[test]
    fn mssql_semicolon_inside_string_cannot_hide_a_write() {
        // Semicolons don't separate T-SQL statements; a quoted `;` used to
        // split the unit and turn the DELETE into a "string".
        assert!(!mssql_read_allowed("SELECT 'x;' DELETE FROM users --'"));
        assert!(!mssql_read_allowed("SELECT ';' ; DELETE FROM users"));
        assert!(!mssql_read_allowed("SELECT [a;b] DROP TABLE t"));
        assert!(mssql_read_allowed("SELECT 'a;b' AS v; SELECT 2"));
        assert!(mssql_read_allowed("SELECT 1 /* ; DELETE */ AS v"));
    }

    // ---- PostgreSQL gate ----

    #[test]
    fn postgres_allows_plain_reads() {
        assert!(postgres_read_allowed("SELECT * FROM users"));
        assert!(postgres_read_allowed(
            "WITH c AS (SELECT 1 AS x) SELECT CASE WHEN x = 1 THEN 'a' END FROM c;"
        ));
        assert!(postgres_read_allowed("SELECT 'SET x = 1; BEGIN' AS s"));
        assert!(postgres_read_allowed("SELECT $$ COMMIT $$ AS s"));
        assert!(postgres_read_allowed("EXPLAIN SELECT 1"));
        assert!(postgres_read_allowed(
            "SELECT offset_col, setting FROM pg_settings"
        ));
    }

    #[test]
    fn postgres_denies_leaving_read_only_mode() {
        assert!(!postgres_read_allowed(
            "SET default_transaction_read_only = off"
        ));
        assert!(!postgres_read_allowed(
            "SET default_transaction_read_only = off; DROP TABLE orders;"
        ));
        assert!(!postgres_read_allowed(
            "BEGIN READ WRITE; DELETE FROM orders; COMMIT;"
        ));
        assert!(!postgres_read_allowed("START TRANSACTION READ WRITE"));
        assert!(!postgres_read_allowed(
            "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE"
        ));
        assert!(!postgres_read_allowed(
            "SELECT set_config('default_transaction_read_only', 'off', false)"
        ));
        assert!(!postgres_read_allowed(
            "SELECT pg_catalog.\"set_config\"('default_transaction_read_only', 'off', false)"
        ));
        assert!(!postgres_read_allowed(
            "SELECT U&\"\\0073et_config\"('a', 'b', false)"
        ));
        assert!(!postgres_read_allowed("RESET ALL"));
        assert!(!postgres_read_allowed("COPY t TO PROGRAM 'calc.exe'"));
        assert!(!postgres_read_allowed("DO $$ BEGIN PERFORM 1; END $$"));
        assert!(!postgres_read_allowed("CALL do_stuff()"));
        assert!(!postgres_read_allowed(
            "SELECT dblink_exec('dbname=x', 'DROP TABLE t')"
        ));
        assert!(!postgres_read_allowed("SELECT pg_terminate_backend(123)"));
    }

    #[test]
    fn postgres_denies_multi_statement_units() {
        // A unit only holds several statements when they share a session;
        // at `read` every statement must run on its own connection.
        assert!(!postgres_read_allowed(
            "DECLARE c CURSOR FOR SELECT 1; FETCH ALL FROM c;"
        ));
    }

    // ---- MongoDB gate ----

    #[test]
    fn mongo_allows_reads() {
        assert_eq!(mongo_read_allowed("db.users.find({})"), Some(true));
        assert_eq!(
            mongo_read_allowed("db.orders.aggregate([{$match: {}}])"),
            Some(true)
        );
        assert_eq!(mongo_read_allowed("db.stats()"), Some(true));
        assert_eq!(mongo_read_allowed("db.getCollectionNames()"), Some(true));
        assert_eq!(mongo_read_allowed("show dbs"), Some(true));
        assert_eq!(mongo_read_allowed("use shop"), Some(true));
    }

    #[test]
    fn mongo_denies_writes_and_arbitrary_commands() {
        assert_eq!(
            mongo_read_allowed("db.users.insertOne({a: 1})"),
            Some(false)
        );
        assert_eq!(
            mongo_read_allowed("db.users.updateMany({}, {$set: {a: 1}})"),
            Some(false)
        );
        assert_eq!(mongo_read_allowed("db.users.deleteMany({})"), Some(false));
        assert_eq!(
            mongo_read_allowed("db.runCommand({dropDatabase: 1})"),
            Some(false)
        );
        assert_eq!(
            mongo_read_allowed("db.getSiblingDB('admin').adminCommand({shutdown: 1})"),
            Some(false)
        );
    }

    #[test]
    fn mongo_denies_write_pipeline_stages() {
        assert_eq!(
            mongo_read_allowed("db.users.aggregate([{$match: {_id: null}}, {$out: \"users\"}])"),
            Some(false)
        );
        assert_eq!(
            mongo_read_allowed("db.users.aggregate([{$merge: {into: \"archive\"}}])"),
            Some(false)
        );
        assert_eq!(
            mongo_read_allowed("db.users.aggregate([{\"$out\": \"users\"}])"),
            Some(false)
        );
        // Nested inside $facet / $lookup sub-pipelines.
        assert_eq!(
            mongo_read_allowed("db.users.aggregate([{$facet: {a: [{$out: \"x\"}]}}])"),
            Some(false)
        );
        assert_eq!(
            mongo_read_allowed("db.users.aggregate([{$match: {note: \"$out\"}}])"),
            Some(true)
        );
    }

    #[test]
    fn mongo_sql_subset_and_garbage_are_undecidable() {
        assert_eq!(mongo_read_allowed("SELECT a FROM b WHERE c = 1"), None);
        assert_eq!(mongo_read_allowed("this is not mongo shell"), None);
    }

    // ---- Access parsing ----

    #[test]
    fn access_parse_is_fail_closed() {
        assert_eq!(Access::parse("read"), Access::Read);
        assert_eq!(Access::parse("write"), Access::Write);
        assert_eq!(Access::parse("off"), Access::Off);
        assert_eq!(Access::parse(""), Access::Off);
        assert_eq!(Access::parse("Write"), Access::Off); // case-sensitive on purpose
        assert_eq!(Access::parse("garbage"), Access::Off);
    }
}
