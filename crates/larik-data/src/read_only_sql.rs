//! Fail-closed static read-only SQL classifier mirroring WinUI's `Larik.Data.ReadOnlySql`.
//!
//! Used as Layer 1 of Larik's two-layer read-only guard for MCP `Read` workspaces
//! (Layer 2 is engine-level transaction rollback / `BEGIN TRANSACTION READ ONLY`).

/// Keyword lists used by [`is_proven_read`] to classify statements for a specific SQL dialect.
#[derive(Debug, Clone, Copy)]
pub struct ReadOnlyKeywordRules {
    /// Allowed leading keywords for every statement unit (case-insensitive).
    pub leading_keywords: &'static [&'static str],
    /// Keywords forbidden anywhere outside quoted literals/identifiers/comments (case-insensitive).
    pub denied_keywords: &'static [&'static str],
    /// Token prefixes forbidden anywhere outside quoted literals/identifiers/comments (case-insensitive).
    pub denied_prefixes: &'static [&'static str],
}

/// Provider-owned static read-only statement policy.
/// Providers without an explicit policy fail closed (`DatabaseProvider::read_only_policy` returns `None`).
pub trait ReadOnlyStatementPolicy: Send + Sync {
    fn is_proven_read(&self, sql: &str) -> bool;
}

/// Returns `true` only when `sql` contains at least one non-empty statement unit and every unit
/// starts with an allowed leading keyword and contains no denied keyword or denied prefix.
///
/// Any unterminated string literal, quoted identifier, bracketed identifier, or block comment
/// fails closed (`false`).
pub fn is_proven_read(sql: &str, rules: &ReadOnlyKeywordRules) -> bool {
    if sql.trim().is_empty() {
        return false;
    }
    let Some(stripped) = try_strip(sql) else {
        return false;
    };

    let mut units = 0usize;
    for raw_unit in stripped.split(';') {
        let tokens = tokenize_words(raw_unit);
        if tokens.is_empty() {
            continue;
        }
        units += 1;

        let first = tokens[0];
        if !rules
            .leading_keywords
            .iter()
            .any(|kw| kw.eq_ignore_ascii_case(first))
        {
            return false;
        }

        for token in tokens {
            if rules
                .denied_keywords
                .iter()
                .any(|kw| kw.eq_ignore_ascii_case(token))
            {
                return false;
            }
            if rules.denied_prefixes.iter().any(|prefix| {
                token.len() >= prefix.len() && token[..prefix.len()].eq_ignore_ascii_case(prefix)
            }) {
                return false;
            }
        }
    }

    units > 0
}

/// Strips `'...'`, `E'...'`, `"..."`, `[...]`, `$tag$...$tag$`, `-- ...`, and nested `/* ... */`,
/// replacing each stripped span with a single space so adjacent tokens cannot merge.
/// Returns `None` on any unterminated quote, bracket, dollar-quote, or block comment.
fn try_strip(sql: &str) -> Option<String> {
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;

    while i < len {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();

        // Line comment: -- ... \n
        if ch == '-' && next == Some('-') {
            i += 2;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            out.push(' ');
            continue;
        }

        // Block comment: /* ... */ (supports nesting as in PostgreSQL & T-SQL, fail-closed if unterminated)
        if ch == '/' && next == Some('*') {
            i += 2;
            let mut depth = 1usize;
            while i < len && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth > 0 {
                return None;
            }
            out.push(' ');
            continue;
        }

        // PostgreSQL dollar-quoted literal: $$...$$ or $tag$...$tag$
        if ch == '$' {
            if let Some(tag_len) = parse_dollar_tag_chars(&chars, i) {
                let tag = &chars[i..i + tag_len];
                i += tag_len;
                let mut closed = false;
                while i + tag_len <= len {
                    if &chars[i..i + tag_len] == tag {
                        i += tag_len;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    return None;
                }
                out.push(' ');
                continue;
            }
        }

        // PostgreSQL escape string literal: E'...' or e'...'
        if (ch == 'E' || ch == 'e')
            && next == Some('\'')
            && (i == 0 || !is_word_part(chars[i - 1]))
        {
            i += 2;
            let mut closed = false;
            while i < len {
                if chars[i] == '\\' {
                    if i + 1 >= len {
                        return None;
                    }
                    i += 2;
                    continue;
                }
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return None;
            }
            out.push(' ');
            continue;
        }

        // Single-quoted literal, double-quoted identifier, or T-SQL bracketed identifier.
        if ch == '\'' || ch == '"' || ch == '[' {
            let closer = if ch == '[' { ']' } else { ch };
            i += 1;
            let mut closed = false;
            while i < len {
                if chars[i] == closer {
                    if chars.get(i + 1) == Some(&closer) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return None;
            }
            out.push(' ');
            continue;
        }

        out.push(ch);
        i += 1;
    }

    Some(out)
}

fn parse_dollar_tag_chars(chars: &[char], pos: usize) -> Option<usize> {
    if chars.get(pos) != Some(&'$') {
        return None;
    }
    if chars.get(pos + 1) == Some(&'$') {
        return Some(2);
    }
    let first = *chars.get(pos + 1)?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    let mut j = pos + 2;
    while j < chars.len() {
        let c = chars[j];
        if c == '$' {
            return Some(j - pos + 1);
        }
        if c.is_alphanumeric() || c == '_' {
            j += 1;
        } else {
            return None;
        }
    }
    None
}

fn is_word_start(ch: char) -> bool {
    ch.is_alphabetic() || matches!(ch, '_' | '@' | '#')
}

fn is_word_part(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '@' | '#' | '$')
}

fn tokenize_words(unit: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut iter = unit.char_indices().peekable();
    while let Some((start, ch)) = iter.next() {
        if !is_word_start(ch) {
            continue;
        }
        let mut end = start + ch.len_utf8();
        while let Some(&(idx, next_ch)) = iter.peek() {
            if is_word_part(next_ch) {
                end = idx + next_ch.len_utf8();
                iter.next();
            } else {
                break;
            }
        }
        tokens.push(&unit[start..end]);
    }
    tokens
}

pub const TSQL_READ_ONLY_RULES: ReadOnlyKeywordRules = ReadOnlyKeywordRules {
    leading_keywords: &["select", "with"],
    denied_keywords: &[
        "insert",
        "update",
        "delete",
        "merge",
        "into",
        "exec",
        "execute",
        "alter",
        "create",
        "drop",
        "truncate",
        "grant",
        "revoke",
        "deny",
        "use",
        "set",
        "declare",
        "begin",
        "commit",
        "rollback",
        "save",
        "backup",
        "restore",
        "dbcc",
        "kill",
        "shutdown",
        "waitfor",
        "reconfigure",
        "checkpoint",
        "bulk",
        "openrowset",
        "openquery",
        "opendatasource",
        "openxml",
        "enable",
        "disable",
        "receive",
        "send",
        "print",
        "raiserror",
    ],
    denied_prefixes: &["xp_"],
};

pub const POSTGRES_READ_ONLY_RULES: ReadOnlyKeywordRules = ReadOnlyKeywordRules {
    leading_keywords: &["select", "with", "values", "table"],
    denied_keywords: &[
        "insert",
        "update",
        "delete",
        "merge",
        "into",
        "copy",
        "create",
        "alter",
        "drop",
        "truncate",
        "grant",
        "revoke",
        "comment",
        "security",
        "set",
        "reset",
        "begin",
        "commit",
        "rollback",
        "savepoint",
        "release",
        "prepare",
        "execute",
        "deallocate",
        "declare",
        "fetch",
        "move",
        "close",
        "listen",
        "notify",
        "unlisten",
        "lock",
        "vacuum",
        "analyze",
        "cluster",
        "reindex",
        "refresh",
        "discard",
        "do",
        "call",
        "load",
        "nextval",
        "setval",
        "set_config",
        "lo_import",
        "lo_export",
        "lo_unlink",
        "dblink",
        "pg_terminate_backend",
        "pg_cancel_backend",
        "pg_reload_conf",
        "pg_switch_wal",
        "pg_create_restore_point",
        "pg_promote",
        "pg_start_backup",
        "pg_stop_backup",
    ],
    denied_prefixes: &[],
};

pub const DEMO_READ_ONLY_RULES: ReadOnlyKeywordRules = ReadOnlyKeywordRules {
    leading_keywords: &["select", "with"],
    denied_keywords: &[
        "insert",
        "update",
        "delete",
        "merge",
        "into",
        "exec",
        "execute",
        "create",
        "alter",
        "drop",
        "truncate",
        "grant",
        "revoke",
        "set",
        "use",
        "begin",
        "commit",
        "rollback",
        "declare",
        "print",
    ],
    denied_prefixes: &[],
};

#[derive(Debug, Default, Clone, Copy)]
pub struct TSqlReadOnlyPolicy;

impl ReadOnlyStatementPolicy for TSqlReadOnlyPolicy {
    fn is_proven_read(&self, sql: &str) -> bool {
        is_proven_read(sql, &TSQL_READ_ONLY_RULES)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PostgresReadOnlyPolicy;

impl ReadOnlyStatementPolicy for PostgresReadOnlyPolicy {
    fn is_proven_read(&self, sql: &str) -> bool {
        is_proven_read(sql, &POSTGRES_READ_ONLY_RULES)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DemoReadOnlyPolicy;

impl ReadOnlyStatementPolicy for DemoReadOnlyPolicy {
    fn is_proven_read(&self, sql: &str) -> bool {
        is_proven_read(sql, &DEMO_READ_ONLY_RULES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsql_allows_selects_ctes_and_quoted_keywords() {
        let p = TSqlReadOnlyPolicy;
        assert!(p.is_proven_read("SELECT 1"));
        assert!(p.is_proven_read("  -- header comment\nSELECT TOP 10 * FROM dbo.orders;"));
        assert!(p.is_proven_read("WITH cte AS (SELECT id FROM dbo.orders) SELECT * FROM cte"));
        assert!(p.is_proven_read("SELECT 'DELETE FROM users' AS note, [into] FROM dbo.orders"));
        assert!(p.is_proven_read("/* drop table */ SELECT 1; SELECT 2;"));
        assert!(p.is_proven_read("SELECT 'it''s fine', \"exec\" FROM dbo.orders"));
    }

    #[test]
    fn tsql_rejects_writes_session_control_and_unterminated_literals() {
        let p = TSqlReadOnlyPolicy;
        assert!(!p.is_proven_read(""));
        assert!(!p.is_proven_read("   ; -- only comment"));
        assert!(!p.is_proven_read("INSERT INTO dbo.orders VALUES (1)"));
        assert!(!p.is_proven_read("UPDATE dbo.orders SET status = 'x'"));
        assert!(!p.is_proven_read("DELETE FROM dbo.orders"));
        assert!(!p.is_proven_read("SELECT * INTO #tmp FROM dbo.orders"));
        assert!(!p.is_proven_read("WITH cte AS (SELECT 1) DELETE FROM dbo.orders"));
        assert!(!p.is_proven_read("SELECT 1; DROP TABLE dbo.orders"));
        assert!(!p.is_proven_read("EXEC xp_cmdshell 'whoami'"));
        assert!(!p.is_proven_read("SELECT xp_something()"));
        assert!(!p.is_proven_read("USE master; SELECT 1"));
        assert!(!p.is_proven_read("SET NOCOUNT ON; SELECT 1"));
        assert!(!p.is_proven_read("SELECT 'unterminated"));
        assert!(!p.is_proven_read("SELECT [unterminated"));
        assert!(!p.is_proven_read("SELECT /* unterminated"));
    }

    #[test]
    fn postgres_rules_handle_values_table_and_side_effects() {
        let p = PostgresReadOnlyPolicy;
        assert!(p.is_proven_read("VALUES (1, 2)"));
        assert!(p.is_proven_read("TABLE public.orders"));
        assert!(p.is_proven_read("SELECT $$DROP TABLE users;$$ AS note"));
        assert!(p.is_proven_read("SELECT $tag$DELETE FROM users;$tag$ AS note"));
        assert!(p.is_proven_read("SELECT E'it\\'s a DROP TABLE test'"));
        assert!(p.is_proven_read("/* outer /* inner DELETE */ still comment */ SELECT 1"));
        assert!(!p.is_proven_read("SELECT $$unterminated"));
        assert!(!p.is_proven_read("SELECT $tag$unterminated"));
        assert!(!p.is_proven_read("SELECT E'unterminated\\'"));
        assert!(!p.is_proven_read("/* outer /* inner */ unterminated SELECT 1"));
        assert!(!p.is_proven_read("DO $$ BEGIN DELETE FROM users; END $$"));
        assert!(!p.is_proven_read("SELECT nextval('orders_id_seq')"));
        assert!(!p.is_proven_read("SELECT set_config('search_path', 'public', false)"));
        assert!(!p.is_proven_read("SELECT pg_terminate_backend(123)"));
    }
}
