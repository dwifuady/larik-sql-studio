// PostgreSQL statement parsing
//
// Splits editor text into statements on `;` (outside strings, quoted
// identifiers, dollar-quoted strings and comments) and classifies statements
// that must go through the simple query protocol (utility commands that
// cannot run inside the implicit transaction block of the extended protocol).

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
    let tokens = top_level_tokens(sql);
    if tokens.is_empty() {
        return StatementKind::ExtendedProtocol;
    }
    let upper: Vec<String> = tokens.iter().map(|t| t.to_ascii_uppercase()).collect();
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

/// Whitespace-separated tokens outside strings, comments and dollar quotes.
fn top_level_tokens(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        if rest.starts_with("--") {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if rest.starts_with("/*") {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        if chars[i] == '\'' {
            i = skip_quoted(&chars, i, '\'');
            if !current.is_empty() {
                current.push(' ');
            }
            continue;
        }
        if chars[i] == '"' || chars[i] == '`' {
            i = skip_quoted(&chars, i, chars[i]);
            if !current.is_empty() {
                current.push(' ');
            }
            continue;
        }
        if chars[i] == '$' {
            let tag_len = dollar_tag_len(&rest);
            if tag_len > 0 {
                // Skip to the matching closing tag (the opening tag itself).
                let close = &rest[tag_len - 1..]; // `$tag$` tail including leading `$`
                let needle_len = tag_len;
                let after = &rest[needle_len..];
                i = match after.find(close) {
                    Some(pos) => i + needle_len + pos + needle_len,
                    None => chars.len(),
                };
                if !current.is_empty() {
                    current.push(' ');
                }
                continue;
            }
            // Not a quote opener (`$1` parameter etc.) — falls through.
        }
        if chars[i].is_whitespace() {
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

/// Skip a `'`- or `"`-quoted run starting at `start` (inclusive of the opener),
/// handling doubled quotes. Returns the index just past the closer.
fn skip_quoted(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
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

/// Length of a dollar-quote opening tag (`$$` or `$tag$`), 0 when the `$` is
/// not a quote opener (e.g. a `$1` parameter placeholder).
fn dollar_tag_len(rest: &str) -> usize {
    if rest.starts_with("$$") {
        return 2;
    }
    let chars: Vec<char> = rest.chars().collect();
    let mut len = 1; // byte position after the opening `$` (ASCII)
    for &c in &chars[1..] {
        match c {
            '$' => {
                let body = &rest[1..len];
                let valid = !body.is_empty()
                    && body
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_alphabetic() || c == '_')
                        .unwrap_or(false)
                    && body
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_');
                return if valid { len + 1 } else { 0 };
            }
            c if c.is_ascii_alphanumeric() || c == '_' => len += 1,
            _ => return 0,
        }
    }
    0
}

/// Parse SQL text into individual statements.
///
/// Splits on `;` outside single-quoted strings (`''` escaped), double-quoted
/// identifiers, dollar-quoted strings (`$$...$$`, `$tag$...$tag$`), line and
/// block comments. The terminator is retained to match the other drivers'
/// behavior. Whitespace/comment-only fragments are dropped.
pub(super) fn parse_sql_statements(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        if rest.starts_with("--") {
            while i < chars.len() && chars[i] != '\n' {
                current.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if rest.starts_with("/*") {
            current.push_str("/*");
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                current.push(chars[i]);
                i += 1;
            }
            if i + 1 < chars.len() {
                current.push_str("*/");
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        if chars[i] == '\'' {
            let end = skip_quoted(&chars, i, '\'');
            current.extend(chars[i..end].iter());
            i = end;
            continue;
        }
        if chars[i] == '"' {
            let end = skip_quoted(&chars, i, '"');
            current.extend(chars[i..end].iter());
            i = end;
            continue;
        }
        if chars[i] == '$' {
            let tag_len = dollar_tag_len(&rest);
            if tag_len > 0 {
                let opening: String = rest[..tag_len].to_string();
                let after = &rest[tag_len..];
                let end = match after.find(&opening) {
                    Some(pos) => i + tag_len + pos + tag_len,
                    None => chars.len(),
                };
                current.extend(chars[i..end].iter());
                i = end;
                continue;
            }
            // `$1` parameter placeholder — ordinary character.
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
    statements
        .into_iter()
        .filter(|s| !is_blank(s))
        .collect()
}

/// Whitespace/comment-only and bare-`;` fragments are not statements.
fn is_blank(s: &str) -> bool {
    let mut rest = s.trim();
    loop {
        if rest.is_empty() {
            return true;
        }
        if let Some(t) = rest.strip_prefix(';') {
            rest = t.trim_start();
            continue;
        }
        if let Some(t) = rest.strip_prefix("--") {
            rest = t.split_once('\n').map(|(_, tail)| tail).unwrap_or("").trim_start();
            continue;
        }
        if let Some(t) = rest.strip_prefix("/*") {
            match t.find("*/") {
                Some(end) => rest = t[end + 2..].trim_start(),
                None => return true,
            }
            continue;
        }
        return false;
    }
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
    fn vacuum_uses_simple_protocol() {
        assert_eq!(classify_statement("VACUUM t"), StatementKind::SimpleProtocol);
        assert_eq!(
            classify_statement("vacuum (analyze) public.t"),
            StatementKind::SimpleProtocol
        );
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
    }
}
