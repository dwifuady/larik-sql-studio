//! T-SQL identifier quoting helpers.
//! See docs/DECISION_CONNECTION_STRATEGY.md and docs/CODE_REVIEW.md §3.1.

/// Bracket-quote a T-SQL identifier, escaping `]` → `]]` (the official rule).
/// `my]db` → `[my]]db]`.
pub fn quote_bracket_ident(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}

/// Cheap allowlist: T-SQL identifiers may start with a letter, `_`, `#`, `##`
/// (temp tables), and contain `[A-Za-z0-9_#]`, `@`-prefixed vars handled by callers.
/// Max length 128 (sysname limit). Rejects anything that looks like SQL.
pub fn is_safe_ident(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || matches!(c, '_' | '#' | '@') => {}
        _ => return false,
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '#' | '@' | '$'))
}

/// Validate + quote in one shot. Returns `Err` with a non-revealing message
/// (do not echo the input back to the user — it may be malicious).
pub fn validate_and_quote(s: &str) -> Result<String, String> {
    if !is_safe_ident(s) {
        return Err("Invalid database identifier".into());
    }
    Ok(quote_bracket_ident(s))
}

/// Relaxed check for database names (the `USE` target).
/// SQL Server allows spaces, hyphens, dots and `]` inside bracket-quoted
/// identifiers — `My Db`, `my-db`, `my]db` are all legal. The strict
/// `is_safe_ident` rejects them, but bracket-quoting with `]→]]` still
/// makes them safe as a single token. We allow a wider charset here but
/// still reject statement delimiters (`;`), quotes, comments and control
/// characters — those are the real injection vectors.
pub fn is_safe_relaxed_ident(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    if s.chars().any(|c| c.is_control() || c == '\0') {
        return false;
    }
    // Reject obvious injection characters; everything else is safe once
    // bracket-quoted because `]` is doubled and the whole value is one token.
    if s.contains(';') || s.contains('\'') || s.contains('"') || s.contains('\0') {
        return false;
    }
    // First char must still be a plausible identifier start (letter, _, #, @, [, digit is allowed when bracketed)
    // — we only enforce that it is not whitespace/control/semicolon.
    let first = s.chars().next().unwrap();
    if first.is_whitespace() || matches!(first, ';' | '\'' | '"' | '-') {
        return false;
    }
    true
}

/// Resolve a database name for `USE`. Tries the strict path first; if that
/// fails, falls back to the relaxed bracket-escaped path (with a warning).
/// Injection strings containing `;` / `'` are rejected by *both* paths.
pub fn resolve_database_name(s: &str) -> Result<String, String> {
    if let Ok(q) = validate_and_quote(s) {
        return Ok(q);
    }
    if is_safe_relaxed_ident(s) {
        eprintln!(
            "[warn] Database name required relaxed quoting — consider renaming to an allowlisted identifier: len={}",
            s.len()
        );
        return Ok(quote_bracket_ident(s));
    }
    Err("Invalid database identifier".into())
}

/// Build a bracket-qualified `schema.table` object name for `OBJECT_ID(N'...')`.
/// Both halves are validated and bracket-escaped; inside `N'...'` no further
/// escaping is needed because the allowlist already excludes quote characters.
pub fn qualified_object_name(schema: &str, table: &str) -> Result<String, String> {
    let schema_q = validate_and_quote(schema)?;
    let table_q = validate_and_quote(table)?;
    Ok(format!("{schema_q}.{table_q}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn quotes_simple() {
        assert_eq!(quote_bracket_ident("foo"), "[foo]");
    }
    #[test]
    fn escapes_closing_bracket() {
        assert_eq!(quote_bracket_ident("my]db"), "[my]]db]");
    }
    #[test]
    fn escapes_double_bracket() {
        assert_eq!(quote_bracket_ident("a]]b"), "[a]]]]b]");
    }
    #[test]
    fn safe_ident_accepts_letter() {
        assert!(is_safe_ident("foo"));
    }
    #[test]
    fn safe_ident_accepts_temp() {
        assert!(is_safe_ident("#temp"));
        assert!(is_safe_ident("##glob"));
    }
    #[test]
    fn safe_ident_accepts_at_var() {
        assert!(is_safe_ident("@x"));
    }
    #[test]
    fn safe_ident_rejects_semicolon() {
        assert!(!is_safe_ident("a; DROP"));
    }
    #[test]
    fn safe_ident_rejects_quote() {
        assert!(!is_safe_ident("a'b"));
    }
    #[test]
    fn safe_ident_rejects_empty() {
        assert!(!is_safe_ident(""));
    }
    #[test]
    fn safe_ident_rejects_long() {
        assert!(!is_safe_ident(&"a".repeat(129)));
    }
    #[test]
    fn validate_rejects_injection() {
        assert!(validate_and_quote("x]; DROP TABLE y; --").is_err());
    }
    #[test]
    fn validate_accepts_dbo() {
        assert_eq!(validate_and_quote("dbo").unwrap(), "[dbo]");
    }
    #[test]
    fn validate_accepts_bracketed_db_via_resolve() {
        // Strict validate_and_quote rejects `]`; the relaxed USE path accepts it.
        assert!(validate_and_quote("my]db").is_err());
        assert_eq!(resolve_database_name("my]db").unwrap(), "[my]]db]");
    }
    #[test]
    fn relaxed_rejects_injection_with_semicolon() {
        assert!(!is_safe_relaxed_ident("x]; SELECT 1; --"));
        assert!(resolve_database_name("x]; SELECT 1; --").is_err());
    }
    #[test]
    fn relaxed_accepts_space_and_hyphen() {
        assert_eq!(resolve_database_name("My Db").unwrap(), "[My Db]");
        assert_eq!(resolve_database_name("my-db").unwrap(), "[my-db]");
    }
    #[test]
    fn validate_rejects_leading_digit() {
        assert!(validate_and_quote("1db").is_err());
    }
    #[test]
    fn validate_rejects_space() {
        assert!(validate_and_quote("my db").is_err());
    }
    #[test]
    fn validate_rejects_dash() {
        assert!(validate_and_quote("my-db").is_err());
    }
    // Regression guards for the OBJECT_ID(N'...') builder (Step 1.1c).
    #[test]
    fn object_name_rejects_injection_in_schema() {
        assert!(qualified_object_name("x]; DROP", "t").is_err());
    }
    #[test]
    fn object_name_rejects_injection_in_table() {
        assert!(qualified_object_name("dbo", "x]; SELECT 1; --").is_err());
    }
    #[test]
    fn object_name_rejects_brackets_in_strict_context() {
        // schema/table come from metadata and must be strict identifiers;
        // names containing `]` belong to the relaxed DB-name path only.
        assert!(qualified_object_name("my]s", "my]t").is_err());
    }
    #[test]
    fn object_name_accepts_strict_names() {
        assert_eq!(
            qualified_object_name("dbo", "my_table").unwrap(),
            "[dbo].[my_table]"
        );
    }
}
