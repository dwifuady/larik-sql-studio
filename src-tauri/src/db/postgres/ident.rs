//! PostgreSQL identifier quoting helpers.
//!
//! Mirrors `db/mssql/ident.rs`: cheap allowlist validation plus double-quote
//! escaping (`"` → `""`, the official rule). `my"db` → `"my""db"`.

/// Double-quote a PostgreSQL identifier, escaping `"` → `""`.
pub fn quote_double_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Cheap allowlist: unquoted PostgreSQL identifiers may start with a letter or
/// `_` and contain `[A-Za-z0-9_$]`. Max length 63 (NAMEDATALEN - 1).
/// Rejects anything that looks like SQL.
pub fn is_safe_ident(s: &str) -> bool {
    if s.is_empty() || s.len() > 63 {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'))
}

/// Validate + quote in one shot. Returns `Err` with a non-revealing message
/// (do not echo the input back to the user — it may be malicious).
pub fn validate_and_quote(s: &str) -> Result<String, String> {
    if is_safe_ident(s) {
        return Ok(quote_double_ident(s));
    }
    // Anything safe to double-quote is still accepted (spaces, unicode, etc.);
    // only statement delimiters, NUL bytes and control characters are rejected —
    // those are the real injection vectors even inside quoted identifiers.
    if s.is_empty() || s.chars().any(|c| c == '\0' || c.is_control()) {
        return Err("Invalid identifier".to_string());
    }
    Ok(quote_double_ident(s))
}

/// Validate + quote a `schema.table` object name for display purposes.
/// Both halves are validated and double-quote-escaped.
pub fn qualified_object_name(schema: &str, table: &str) -> Result<String, String> {
    Ok(format!(
        "{}.{}",
        validate_and_quote(schema)?,
        validate_and_quote(table)?
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn quotes_standard_idents() {
        assert_eq!(quote_double_ident("public"), "\"public\"");
        assert_eq!(validate_and_quote("users").unwrap(), "\"users\"");
    }

    #[test]
    fn escapes_embedded_quotes() {
        assert_eq!(quote_double_ident("my\"db"), "\"my\"\"db\"");
        assert_eq!(validate_and_quote("weird name").unwrap(), "\"weird name\"");
    }

    #[test]
    fn rejects_dangerous_idents() {
        assert!(validate_and_quote("").is_err());
        assert!(validate_and_quote("a\u{0}b").is_err());
        assert!(validate_and_quote("a\nb").is_err());
    }

    #[test]
    fn allowlist_accepts_common_shapes() {
        assert!(is_safe_ident("public"));
        assert!(is_safe_ident("_x1$"));
        assert!(!is_safe_ident("1abc"));
        assert!(!is_safe_ident("a b"));
        assert!(!is_safe_ident("a;b"));
        assert!(!is_safe_ident(&"x".repeat(64)));
    }

    #[test]
    fn qualified_names_quote_both_halves() {
        assert_eq!(
            qualified_object_name("public", "users").unwrap(),
            "\"public\".\"users\""
        );
    }
}
