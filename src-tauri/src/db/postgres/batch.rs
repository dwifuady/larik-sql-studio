// PostgreSQL statement parsing
//
// Splits editor text into statements on `;` (outside strings, quoted
// identifiers, dollar-quoted strings and comments), keeps scripts that depend
// on session state together, and classifies statements that must go through
// the simple query protocol (utility commands that cannot run inside the
// implicit transaction block of the extended protocol).
//
// All scanning works on a `&[char]` with char indices — never mixing byte
// offsets in — so multibyte text cannot shift positions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatementKind {
    /// Run through `simple_query` (no implicit transaction, text results).
    SimpleProtocol,
    /// Run through prepare + query/execute (typed rows, streaming).
    ExtendedProtocol,
}

/// True when the statement must use the simple query protocol.
///
/// The extended query protocol wraps every statement in an implicit
/// transaction, which rejects a small set of utility commands:
/// - `VACUUM ...`
/// - `ALTER SYSTEM ...`
/// - `CREATE [UNIQUE] INDEX CONCURRENTLY`, `DROP INDEX CONCURRENTLY`,
///   `REINDEX ... CONCURRENTLY`, `REFRESH MATERIALIZED VIEW CONCURRENTLY`
pub(super) fn classify_statement(sql: &str) -> StatementKind {
    let upper = upper_tokens(sql);
    if upper.is_empty() {
        return StatementKind::ExtendedProtocol;
    }
    let has_concurrently = |range: std::ops::Range<usize>| {
        range
            .filter(|i| *i < upper.len())
            .any(|i| upper[i] == "CONCURRENTLY")
    };

    match upper[0].as_str() {
        "VACUUM" => StatementKind::SimpleProtocol,
        "ALTER" if upper.get(1).map(String::as_str) == Some("SYSTEM") => {
            StatementKind::SimpleProtocol
        }
        "CREATE" | "DROP" => {
            // `[UNIQUE] INDEX CONCURRENTLY` — INDEX within the first three
            // tokens, CONCURRENTLY immediately after it.
            let index_pos = upper.iter().take(3).position(|t| t == "INDEX");
            match index_pos {
                Some(pos) if upper.get(pos + 1).map(String::as_str) == Some("CONCURRENTLY") => {
                    StatementKind::SimpleProtocol
                }
                _ => StatementKind::ExtendedProtocol,
            }
        }
        "REFRESH" => {
            if upper.len() > 3
                && upper[1] == "MATERIALIZED"
                && upper[2] == "VIEW"
                && upper[3] == "CONCURRENTLY"
            {
                StatementKind::SimpleProtocol
            } else {
                StatementKind::ExtendedProtocol
            }
        }
        "REINDEX" => {
            // REINDEX [(options)] {INDEX|TABLE|SCHEMA|...} [CONCURRENTLY] name
            if has_concurrently(1..5) {
                StatementKind::SimpleProtocol
            } else {
                StatementKind::ExtendedProtocol
            }
        }
        _ => StatementKind::ExtendedProtocol,
    }
}

/// Split editor text into the units the `QueryEngine` executes one at a time.
///
/// Each unit runs on its own connection, so statements that share session
/// state (transactions, `SET`, temp tables, cursors, prepared statements)
/// would silently lose it if split — e.g. `BEGIN; DELETE ...; ROLLBACK;`
/// would auto-commit the DELETE. Such scripts stay a single unit; the driver
/// runs its statements in order on one connection.
pub(super) fn split_statements(sql: &str) -> Vec<String> {
    let statements = parse_sql_statements(sql);
    if statements.len() > 1 && statements.iter().any(|s| needs_session(s)) {
        vec![sql.trim().to_string()]
    } else {
        statements
    }
}

/// Whether a statement creates or depends on per-session state that later
/// statements in the same script may rely on.
fn needs_session(sql: &str) -> bool {
    let upper = upper_tokens(sql);
    let Some(first) = upper.first() else {
        return false;
    };
    let session_command = matches!(
        first.as_str(),
        "BEGIN"
            | "START"
            | "COMMIT"
            | "END"
            | "ROLLBACK"
            | "ABORT"
            | "SAVEPOINT"
            | "RELEASE"
            | "PREPARE"
            | "EXECUTE"
            | "DEALLOCATE"
            | "SET"
            | "RESET"
            | "DECLARE"
            | "FETCH"
            | "MOVE"
            | "CLOSE"
            | "LISTEN"
            | "UNLISTEN"
            | "DISCARD"
            | "LOCK"
    );
    if session_command {
        return true;
    }
    // CREATE [GLOBAL|LOCAL] {TEMP|TEMPORARY} ..., CREATE OR REPLACE TEMP VIEW ...
    let is_temp = |t: &String| t == "TEMP" || t == "TEMPORARY";
    if first == "CREATE" && upper.iter().skip(1).take(4).any(is_temp) {
        return true;
    }
    // SELECT ... INTO TEMP[ORARY] t, or explicit pg_temp references
    upper
        .windows(2)
        .any(|w| w[0] == "INTO" && is_temp(&w[1]))
        || upper.iter().any(|t| t.contains("PG_TEMP."))
}

/// Parse SQL text into individual statements.
///
/// Splits on `;` outside single-quoted strings (`''` escaped, plus backslash
/// escapes in `E'...'`), double-quoted identifiers, dollar-quoted strings
/// (`$$...$$`, `$tag$...$tag$`), line and (nested) block comments. The
/// terminator is retained to match the other drivers' behavior.
/// Whitespace/comment-only fragments are dropped.
pub(super) fn parse_sql_statements(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = skip_non_code(&chars, i) {
            current.extend(&chars[i..end]);
            i = end;
            continue;
        }
        if chars[i] == ';' {
            current.push(';');
            statements.push(std::mem::take(&mut current));
            i += 1;
            continue;
        }
        current.push(chars[i]);
        i += 1;
    }
    // A final statement without a trailing `;` still counts.
    statements.push(current);
    statements.into_iter().filter(|s| !is_blank(s)).collect()
}

/// Upper-cased whitespace-separated tokens outside strings, comments and
/// dollar quotes.
fn upper_tokens(sql: &str) -> Vec<String> {
    top_level_tokens(sql)
        .iter()
        .map(|t| t.to_ascii_uppercase())
        .collect()
}

/// Tokens outside strings, comments and dollar quotes, separated by
/// whitespace and `;` `(` `)` `,`. A skipped quoted run also separates.
fn top_level_tokens(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = skip_non_code(&chars, i) {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            i = end;
            continue;
        }
        if chars[i].is_whitespace() || matches!(chars[i], ';' | '(' | ')' | ',') {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            i += 1;
            continue;
        }
        current.push(chars[i]);
        i += 1;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Whitespace/comment-only and bare-`;` fragments are not statements.
fn is_blank(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if starts_with_at(&chars, i, &['-', '-']) || starts_with_at(&chars, i, &['/', '*']) {
            i = skip_non_code(&chars, i).unwrap_or(chars.len());
            continue;
        }
        if chars[i].is_whitespace() || chars[i] == ';' {
            i += 1;
            continue;
        }
        return false;
    }
    true
}

/// If a comment, string, quoted identifier or dollar-quoted string starts at
/// `i`, return the index just past it (or the end of input when unterminated).
fn skip_non_code(chars: &[char], i: usize) -> Option<usize> {
    if starts_with_at(chars, i, &['-', '-']) {
        let mut j = i;
        while j < chars.len() && chars[j] != '\n' {
            j += 1;
        }
        return Some(j);
    }
    if starts_with_at(chars, i, &['/', '*']) {
        return Some(skip_block_comment(chars, i));
    }
    match chars[i] {
        '\'' => {
            let escapes = is_escape_string_prefix(chars, i);
            Some(skip_quoted(chars, i, '\'', escapes))
        }
        '"' => Some(skip_quoted(chars, i, '"', false)),
        '$' => {
            // A `$` continuing an identifier (`a$b`) never opens a quote.
            if i > 0 && is_ident_char(chars[i - 1]) {
                return None;
            }
            let tag_len = dollar_tag_len(chars, i);
            if tag_len == 0 {
                return None; // `$1` parameter placeholder etc.
            }
            let tag = &chars[i..i + tag_len];
            Some(match find_seq(chars, i + tag_len, tag) {
                Some(pos) => pos + tag_len,
                None => chars.len(),
            })
        }
        _ => None,
    }
}

/// Skip a (possibly nested) `/* ... */` comment starting at `start`.
fn skip_block_comment(chars: &[char], start: usize) -> usize {
    let mut depth = 0usize;
    let mut i = start;
    while i < chars.len() {
        if starts_with_at(chars, i, &['/', '*']) {
            depth += 1;
            i += 2;
        } else if starts_with_at(chars, i, &['*', '/']) {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    chars.len()
}

/// `E'...'` / `e'...'` escape-string literal: the quote follows a standalone `E`.
fn is_escape_string_prefix(chars: &[char], quote: usize) -> bool {
    quote > 0
        && matches!(chars[quote - 1], 'E' | 'e')
        && (quote < 2 || !is_ident_char(chars[quote - 2]))
}

/// Skip a quoted run starting at `start` (inclusive of the opener), handling
/// doubled quotes and, for escape strings, backslash escapes. Returns the
/// index just past the closer.
fn skip_quoted(chars: &[char], start: usize, quote: char, backslash_escapes: bool) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if backslash_escapes && chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == quote {
            if i + 1 < chars.len() && chars[i + 1] == quote {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// Length (in chars) of a dollar-quote opening tag (`$$` or `$tag$`) at `i`,
/// 0 when the `$` is not a quote opener (e.g. a `$1` parameter placeholder).
fn dollar_tag_len(chars: &[char], i: usize) -> usize {
    match chars.get(i + 1) {
        Some('$') => return 2,
        Some(&c) if c.is_alphabetic() || c == '_' => {}
        _ => return 0,
    }
    let mut j = i + 2;
    while j < chars.len() {
        match chars[j] {
            '$' => return j - i + 1,
            c if c.is_alphanumeric() || c == '_' => j += 1,
            _ => return 0,
        }
    }
    0
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn starts_with_at(chars: &[char], i: usize, pattern: &[char]) -> bool {
    chars.get(i..i + pattern.len()) == Some(pattern)
}

/// First index `>= from` where `needle` occurs in `chars`.
fn find_seq(chars: &[char], from: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() || from >= chars.len() {
        return None;
    }
    chars[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|pos| from + pos)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_top_level_semicolons() {
        let out = parse_sql_statements("SELECT 1; SELECT 2");
        assert_eq!(out, vec!["SELECT 1;", " SELECT 2"]);
    }

    #[test]
    fn keeps_semicolons_inside_strings_and_dollar_quotes() {
        let out = parse_sql_statements("SELECT 'a;b' AS x, $$y;z$$ AS y FROM t;");
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("'a;b'"));
        assert!(out[0].contains("$$y;z$$"));
    }

    #[test]
    fn handles_tagged_dollar_quotes() {
        let out = parse_sql_statements(
            "SELECT $fn$ CREATE FUNCTION f() RETURNS void AS ';begin;' LANGUAGE plpgsql; $fn$; SELECT 2;",
        );
        assert_eq!(out.len(), 2);
        assert!(out[0].contains("$fn$"));
        assert!(out[0].contains("';begin;'"));
        assert!(out[0].contains("LANGUAGE plpgsql"));
        assert_eq!(out[1].trim(), "SELECT 2;");
    }

    #[test]
    fn multibyte_text_inside_dollar_quotes() {
        // Byte/char offset mixing used to panic or swallow the `;` here.
        assert_eq!(parse_sql_statements("SELECT $$é$$"), vec!["SELECT $$é$$"]);
        let out = parse_sql_statements("SELECT $$日本語;é$$; SELECT 'ü';");
        assert_eq!(out, vec!["SELECT $$日本語;é$$;", " SELECT 'ü';"]);
        let out = parse_sql_statements("DO $body$ BEGIN RAISE NOTICE '→'; END $body$; SELECT 1;");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn escape_strings_honour_backslashes() {
        let out = parse_sql_statements(r"SELECT E'it\'s; fine'; SELECT 2;");
        assert_eq!(out.len(), 2);
        assert!(out[0].contains(r"E'it\'s; fine'"));
        // A plain string ending in a backslash is not an escape.
        let out = parse_sql_statements(r"SELECT 'C:\'; SELECT 2;");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn nested_block_comments() {
        let out = parse_sql_statements("SELECT 1 /* a /* b ; */ c ; */; SELECT 2;");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn identifier_dollar_is_not_a_quote() {
        let out = parse_sql_statements("SELECT a$b$ FROM t; SELECT 2;");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn large_scripts_split_quickly() {
        let script = "INSERT INTO t VALUES (1, 'x');\n".repeat(20_000);
        let start = std::time::Instant::now();
        assert_eq!(parse_sql_statements(&script).len(), 20_000);
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn ignores_comments() {
        let out = parse_sql_statements("SELECT 1 -- not ; a split\n; SELECT 2 /* ; */ ;");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn drops_empty_fragments() {
        let out = parse_sql_statements(";;  ; SELECT 1;");
        assert_eq!(out.len(), 1);
        assert_eq!(parse_sql_statements("-- just a comment"), Vec::<String>::new());
        assert_eq!(parse_sql_statements("/* block */"), Vec::<String>::new());
    }

    #[test]
    fn parameter_placeholders_are_not_dollar_tags() {
        let out = parse_sql_statements("SELECT * FROM t WHERE a = $1 AND b = 'x;y';");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn session_scripts_stay_together() {
        let script = "BEGIN; DELETE FROM t; ROLLBACK;";
        assert_eq!(split_statements(script), vec![script.to_string()]);
        let script = "SET search_path TO x; SELECT * FROM t;";
        assert_eq!(split_statements(script).len(), 1);
        let script = "CREATE TEMP TABLE x (a int); INSERT INTO x VALUES (1); SELECT * FROM x;";
        assert_eq!(split_statements(script).len(), 1);
        let script = "SELECT 1 INTO TEMPORARY t; SELECT * FROM t;";
        assert_eq!(split_statements(script).len(), 1);
    }

    #[test]
    fn independent_statements_split() {
        assert_eq!(split_statements("SELECT 1; UPDATE t SET a = 1;").len(), 2);
        // Keywords inside strings or bodies don't count.
        assert_eq!(
            split_statements("SELECT 'begin'; DO $$ BEGIN PERFORM 1; END $$;").len(),
            2
        );
        // A lone session statement is still a single unit.
        assert_eq!(split_statements("SET search_path TO x;").len(), 1);
    }

    #[test]
    fn vacuum_uses_simple_protocol() {
        assert_eq!(classify_statement("VACUUM t"), StatementKind::SimpleProtocol);
        assert_eq!(
            classify_statement("vacuum (analyze) public.t"),
            StatementKind::SimpleProtocol
        );
        assert_eq!(classify_statement("VACUUM;"), StatementKind::SimpleProtocol);
        assert_eq!(classify_statement("vacuum(analyze)"), StatementKind::SimpleProtocol);
    }

    #[test]
    fn alter_system_uses_simple_protocol() {
        assert_eq!(
            classify_statement("ALTER SYSTEM SET max_connections = '100';"),
            StatementKind::SimpleProtocol
        );
        assert_eq!(
            classify_statement("ALTER TABLE t ADD COLUMN c int"),
            StatementKind::ExtendedProtocol
        );
    }

    #[test]
    fn concurrent_ddl_uses_simple_protocol() {
        assert_eq!(
            classify_statement("CREATE INDEX CONCURRENTLY idx ON t (c)"),
            StatementKind::SimpleProtocol
        );
        assert_eq!(
            classify_statement("CREATE UNIQUE INDEX CONCURRENTLY idx ON t (c)"),
            StatementKind::SimpleProtocol
        );
        assert_eq!(
            classify_statement("REINDEX SCHEMA CONCURRENTLY public"),
            StatementKind::SimpleProtocol
        );
        assert_eq!(
            classify_statement("DROP INDEX CONCURRENTLY public.idx"),
            StatementKind::SimpleProtocol
        );
        assert_eq!(
            classify_statement("REFRESH MATERIALIZED VIEW CONCURRENTLY mv"),
            StatementKind::SimpleProtocol
        );
    }

    #[test]
    fn ordinary_statements_use_extended_protocol() {
        assert_eq!(
            classify_statement("CREATE INDEX idx ON t (c)"),
            StatementKind::ExtendedProtocol
        );
        assert_eq!(
            classify_statement("REFRESH MATERIALIZED VIEW mv"),
            StatementKind::ExtendedProtocol
        );
        assert_eq!(
            classify_statement("SELECT 'create index concurrently' -- vacuum\nFROM t"),
            StatementKind::ExtendedProtocol
        );
        assert_eq!(
            classify_statement("UPDATE t SET a = 1"),
            StatementKind::ExtendedProtocol
        );
    }

    #[test]
    fn tokens_skip_strings_and_comments() {
        assert_eq!(top_level_tokens("SELECT 'a b' -- x\nFROM t"), ["SELECT", "FROM", "t"]);
        assert_eq!(
            top_level_tokens("SELECT $$ x y $$ FROM t"),
            ["SELECT", "FROM", "t"]
        );
    }
}
