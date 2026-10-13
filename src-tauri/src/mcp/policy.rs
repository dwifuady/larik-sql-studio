// Access policy for the embedded MCP server.
//
// Fail-closed rules:
// - A space is visible to MCP only when the user set `mcp_access` to `read`
//   or `write`; unknown stored values fall back to `off`.
// - At access level `read`, statements are gated per engine:
//   - PostgreSQL: engine-level enforcement (`default_transaction_read_only` via
//     startup options on the dedicated session) — no text gate here.
//   - MongoDB: allow-list of read methods on the parsed shell statement.
//   - MS-SQL: no session-level read-only exists in T-SQL, so a conservative
//     classifier denies anything not provably a read. For a hard guarantee use
//     a read-only login; the Settings UI says the same.

use crate::db::mongodb::shell::{parse_single, Statement};

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
        Ok(Statement::DatabaseCall { call, .. }) | Ok(Statement::CollectionCall { call, .. }) => {
            Some(MONGO_READ_METHODS.contains(&call.name.as_str()))
        }
        Err(_) => None,
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
    "INTO",
    "IDENTITY",
    "OUTPUT",
];

/// Classify one already-split T-SQL unit at access level `read`.
///
/// Every semicolon-separated part must start with a read verb and contain no
/// banned token outside string literals, comments, and quoted identifiers.
/// Anything the classifier cannot prove a read is denied — including reads it
/// misparses (e.g. a semicolon inside a string literal splits the unit and the
/// fragments are judged independently). Best-effort by design; for a hard
/// guarantee use a read-only login.
pub fn mssql_read_allowed(unit: &str) -> bool {
    unit.split(';').all(|part| {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            return true;
        }
        let stripped = strip_literals(trimmed);
        let upper = stripped.to_ascii_uppercase();
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
