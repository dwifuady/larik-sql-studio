// T-SQL batch parsing
// Splits editor text into statements (GO / `;` with scope preservation),
// auto-wraps bare procedure calls with EXEC, and classifies statements as DML.

/// Auto-wrap procedure calls without EXEC keyword
/// If a statement looks like a bare procedure name (e.g., "sp_who2", "dbo.sp_who2"),
/// wrap it with EXEC to allow execution without explicit EXEC keyword (like SSMS/DBeaver).
fn auto_wrap_procedure(stmt: String) -> String {
    let trimmed = stmt.trim();

    // If already wrapped with EXEC, return as-is
    if trimmed.to_uppercase().starts_with("EXEC") || trimmed.to_uppercase().starts_with("EXECUTE") {
        return stmt;
    }

    // If it starts with any of these keywords, it's not a bare procedure call
    let reserved_keywords = [
        "SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP", "DECLARE", "SET", "IF",
        "BEGIN", "END", "WHILE", "FOR", "MERGE", "WITH", "UNION", "USE", "PRINT", "RETURN", "CAST",
        "CASE",
    ];

    let first_word = trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_uppercase();

    // If it starts with a reserved keyword, it's not a bare procedure
    if reserved_keywords.iter().any(|&kw| first_word == kw) {
        return stmt;
    }

    // Check if it looks like a procedure name (identifier or schema.identifier)
    // Pattern: [a-zA-Z_][a-zA-Z0-9_]* optionally prefixed with [schema.]
    // and potentially followed by parentheses with parameters
    let is_likely_procedure = trimmed
        .chars()
        .next()
        .map(|c| c.is_alphabetic() || c == '_' || c == '[')
        .unwrap_or(false);

    if is_likely_procedure {
        // Check if it contains spaces (likely a procedure with parameters)
        // or if it's a simple identifier/schema.identifier pattern
        let has_parentheses = trimmed.contains('(');
        let trimmed_upper = trimmed.to_uppercase();

        // If it has WITH, ORDER BY, GROUP BY, WHERE, FROM, or other SQL keywords, it's a query
        if trimmed_upper.contains(" WITH")
            || trimmed_upper.contains(" ORDER BY")
            || trimmed_upper.contains(" GROUP BY")
            || trimmed_upper.contains(" WHERE")
            || trimmed_upper.contains(" FROM")
            || trimmed_upper.contains(" JOIN")
        {
            return stmt;
        }

        // If it looks like a simple procedure call, wrap it with EXEC
        if !has_parentheses
            || trimmed_upper.starts_with("SP_")
            || trimmed_upper.starts_with("DBO.SP_")
            || trimmed_upper.contains(".")
        {
            return format!("EXEC {}", trimmed);
        }
    }

    stmt
}

/// Parse SQL text into individual statements.
///
/// Splitting strategy:
/// 1. Split on the `GO` batch separator (line-level, case-insensitive, only
///    when not inside a string or block comment). `GO` is a true batch boundary
///    in T-SQL — variables and temp tables do not survive across it.
/// 2. Within each `GO` batch, decide whether to also split on semicolons:
///    - If the batch needs scope preservation (contains `DECLARE`, a
///      statement-start `SET`, or `#`/`##` temp-table references outside
///      strings/comments/brackets), keep the WHOLE batch as a single statement
///      so it is sent to the server in one TDS request. Splitting such a batch
///      would either lose variable/temp-table scope or produce spurious empty
///      result tabs (one per non-rowset statement).
///    - Otherwise split on `;` so independent statements each get their own
///      result tab.
///
/// Auto-wrapping: if a statement is just a procedure name (e.g., "sp_who2"),
/// it's automatically wrapped with EXEC to allow execution without an explicit
/// EXEC keyword (like SSMS/DBeaver).
pub(super) fn parse_sql_statements(sql: &str) -> Vec<String> {
    let trimmed_sql = sql.trim();
    if trimmed_sql.is_empty() {
        return Vec::new();
    }

    // Phase 1: split into batches on GO.
    let go_batches = split_on_go(trimmed_sql);

    // Phase 2: per batch, either keep whole (scope-sensitive) or split on `;`.
    let mut statements = Vec::new();
    for batch in go_batches {
        let batch = batch.trim();
        if batch.is_empty() {
            continue;
        }

        if batch_needs_scope_preservation(batch) {
            statements.push(auto_wrap_procedure(batch.to_string()));
        } else {
            for s in split_on_semicolon(batch) {
                let stmt = s.trim();
                if !stmt.is_empty() && stmt != ";" {
                    statements.push(auto_wrap_procedure(s));
                }
            }
        }
    }

    // If nothing parsed, return the original as a single statement.
    if statements.is_empty() && !trimmed_sql.is_empty() {
        statements.push(auto_wrap_procedure(trimmed_sql.to_string()));
    }

    statements
}

/// Split SQL on the `GO` batch separator (`GO` on its own line,
/// case-insensitive), ignoring `GO` inside strings or block comments.
fn split_on_go(sql: &str) -> Vec<String> {
    let mut batches = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut in_block_comment = false;
    let mut in_line_comment = false;
    let mut prev_char = '\0';

    for line in sql.lines() {
        let trimmed = line.trim();

        // GO is a separator only when not inside a string/block comment/line
        // comment at the start of the line.
        let go_separates = !in_string
            && !in_block_comment
            && !in_line_comment
            && trimmed.eq_ignore_ascii_case("go");

        if go_separates {
            let batch = current.trim().to_string();
            if !batch.is_empty() {
                batches.push(batch);
            }
            current.clear();
            // Reset state for the next batch — GO cannot be mid-string/comment.
            in_string = false;
            in_block_comment = false;
            in_line_comment = false;
            prev_char = '\0';
            continue;
        }

        // Scan the line to keep string/comment state accurate.
        for ch in line.chars() {
            if ch == '\'' && prev_char != '\\' && !in_block_comment && !in_line_comment {
                in_string = !in_string;
            }
            if !in_string {
                if ch == '-' && prev_char == '-' && !in_block_comment {
                    in_line_comment = true;
                } else if ch == '*' && prev_char == '/' && !in_line_comment {
                    in_block_comment = true;
                } else if ch == '/' && prev_char == '*' && in_block_comment {
                    in_block_comment = false;
                }
            }
            prev_char = ch;
        }

        in_line_comment = false;
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    let final_batch = current.trim().to_string();
    if !final_batch.is_empty() {
        batches.push(final_batch);
    }

    if batches.is_empty() && !sql.trim().is_empty() {
        batches.push(sql.trim().to_string());
    }

    batches
}

/// Split a SQL batch on semicolons that are outside strings, line comments, and
/// block comments. Returns each non-empty statement (terminator retained on the
/// statement text to match historical behavior).
fn split_on_semicolon(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut in_block_comment = false;
    let mut in_line_comment = false;
    let mut prev_char = '\0';

    for line in sql.lines() {
        for ch in line.chars() {
            if ch == '\'' && prev_char != '\\' && !in_block_comment && !in_line_comment {
                in_string = !in_string;
            }
            if !in_string {
                if ch == '-' && prev_char == '-' && !in_block_comment {
                    in_line_comment = true;
                } else if ch == '*' && prev_char == '/' && !in_line_comment {
                    in_block_comment = true;
                } else if ch == '/' && prev_char == '*' && in_block_comment {
                    in_block_comment = false;
                }
            }

            if ch == ';' && !in_string && !in_block_comment && !in_line_comment {
                current.push(ch);
                let stmt = current.trim().to_string();
                if !stmt.is_empty() && stmt != ";" {
                    statements.push(stmt);
                }
                current.clear();
                prev_char = ch;
                continue;
            }

            current.push(ch);
            prev_char = ch;
        }
        in_line_comment = false;
        current.push('\n');
    }

    let final_stmt = current.trim().to_string();
    if !final_stmt.is_empty() && final_stmt != ";" {
        statements.push(final_stmt);
    }

    statements
}

/// Detect whether a SQL batch must be sent to the server as a single statement
/// to preserve scope. Returns true when the batch (outside strings, comments,
/// and `[...]` bracketed identifiers) contains any of:
///
/// - `DECLARE` at the start of a statement (after `;` or at batch start)
/// - `SET` at the start of a statement (covers session SETs like NOCOUNT as
///   well as variable assignments — splitting those off otherwise yields
///   spurious empty result tabs)
/// - A `#` or `##` temp-table reference (`#` followed by an identifier char)
///
/// Each signals that the batch carries state (variable or temp-table scope, or
/// a session SET that should not produce its own empty result tab) across `;`
/// boundaries within the batch.
fn batch_needs_scope_preservation(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0usize;
    let mut in_string = false;
    let mut in_block_comment = false;
    let mut in_line_comment = false;
    let mut statement_start = true;

    while i < bytes.len() {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied();

        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if b == b'*' && next == Some(b'/') {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_string {
            if b == b'\'' {
                if next == Some(b'\'') {
                    i += 2;
                } else {
                    in_string = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        // Not in string/comment — detect comment starts.
        if b == b'-' && next == Some(b'-') {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if b == b'/' && next == Some(b'*') {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if b == b'\'' {
            in_string = true;
            i += 1;
            continue;
        }

        // Semicolons end a statement; whitespace doesn't change statement_start.
        if b == b';' {
            statement_start = true;
            i += 1;
            continue;
        }
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // `[...]` bracketed identifiers — skip contents (a `#` inside brackets
        // is part of an identifier name, e.g. `[#count]` column alias, not a
        // temp table reference).
        if b == b'[' {
            i += 1;
            while i < bytes.len() && bytes[i] != b']' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // consume `]`
            }
            statement_start = false;
            continue;
        }

        // Temp-table reference: `#` or `##` followed by an identifier char,
        // outside strings/comments/brackets.
        if b == b'#' {
            let after_hash = if next == Some(b'#') {
                bytes.get(i + 2).copied()
            } else {
                next
            };
            if let Some(c) = after_hash {
                if c.is_ascii_alphanumeric() || c == b'_' {
                    return true;
                }
            }
            statement_start = false;
            i += 1;
            continue;
        }

        // Identifier-start at a statement boundary — read the keyword.
        if statement_start && (b.is_ascii_alphabetic() || b == b'_') {
            let start = i;
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let upper = sql[start..i].to_uppercase();
            if upper == "DECLARE" {
                return true;
            }
            if upper == "SET" {
                // Any statement-start SET: covers session SETs (NOCOUNT,
                // ROWCOUNT) that would otherwise produce spurious empty
                // tabs, and SET @var assignments.
                return true;
            }
            statement_start = false;
            continue;
        }

        // Any other non-whitespace token ends the statement-start window.
        statement_start = false;
        i += 1;
    }

    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatementKind {
    Dml,
    Other,
}

fn starts_with_keyword(input: &str, keyword: &str) -> bool {
    if !input.starts_with(keyword) {
        return false;
    }

    match input.as_bytes().get(keyword.len()) {
        None => true,
        Some(next) => !next.is_ascii_alphanumeric() && *next != b'_',
    }
}

pub(super) fn infer_statement_kind(sql: &str) -> StatementKind {
    let mut last_kind = StatementKind::Other;

    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut statement_start = true;

    let bytes = sql.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied();

        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }

        if in_block_comment {
            if b == b'*' && next == Some(b'/') {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if in_string {
            if b == b'\'' {
                if next == Some(b'\'') {
                    i += 2;
                } else {
                    in_string = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        if b == b'-' && next == Some(b'-') {
            in_line_comment = true;
            i += 2;
            continue;
        }

        if b == b'/' && next == Some(b'*') {
            in_block_comment = true;
            i += 2;
            continue;
        }

        if b == b'\'' {
            in_string = true;
            i += 1;
            continue;
        }

        if b == b';' {
            statement_start = true;
            i += 1;
            continue;
        }

        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if statement_start {
            if b.is_ascii_alphabetic() || b == b'_' {
                let start = i;
                i += 1;
                while i < bytes.len() {
                    let c = bytes[i];
                    if c.is_ascii_alphanumeric() || c == b'_' {
                        i += 1;
                    } else {
                        break;
                    }
                }

                let upper = sql[start..i].to_uppercase();

                if starts_with_keyword(&upper, "UPDATE")
                    || starts_with_keyword(&upper, "INSERT")
                    || starts_with_keyword(&upper, "DELETE")
                    || starts_with_keyword(&upper, "MERGE")
                {
                    last_kind = StatementKind::Dml;
                } else if starts_with_keyword(&upper, "SELECT")
                    || starts_with_keyword(&upper, "EXEC")
                    || starts_with_keyword(&upper, "EXECUTE")
                    || starts_with_keyword(&upper, "CREATE")
                    || starts_with_keyword(&upper, "ALTER")
                    || starts_with_keyword(&upper, "DROP")
                    || starts_with_keyword(&upper, "TRUNCATE")
                    || starts_with_keyword(&upper, "WITH")
                {
                    last_kind = StatementKind::Other;
                }
            } else {
                i += 1;
            }

            statement_start = false;
            continue;
        }

        i += 1;
    }

    last_kind
}

#[cfg(test)]
mod tests {
    use super::{
        batch_needs_scope_preservation, infer_statement_kind, parse_sql_statements, StatementKind,
    };

    #[test]
    fn infer_kind_for_declare_then_update_is_dml() {
        let sql = "\
DECLARE @ExistingChildId int = (SELECT Id FROM ProductQuestions WHERE ShortName = 'CLMoreThan25FromUSOrCanada');
UPDATE ProductQuestion_Rule
SET EffectiveFrom = '2026-05-01'
WHERE ProductQuestionId = 2856;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Dml);
    }

    #[test]
    fn infer_kind_for_update_with_subquery_select_is_dml() {
        let sql = "\
UPDATE ProductQuestion_Rule
SET EffectiveFrom = (SELECT MAX(EffectiveFrom) FROM ProductQuestion_Rule)
WHERE ProductQuestionId = 2856;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Dml);
    }

    #[test]
    fn infer_kind_ignores_comments_and_uses_last_executable_statement() {
        let sql = "\
/* UPDATE ProductQuestion_Rule SET EffectiveFrom = '1900-01-01' */
-- DELETE FROM ProductQuestion_Rule WHERE ProductQuestionId = 2856
DECLARE @note nvarchar(100) = 'comment mentions UPDATE';
UPDATE ProductQuestion_Rule SET EffectiveFrom = '2026-05-01' WHERE ProductQuestionId = 2856;
SELECT EffectiveFrom FROM ProductQuestion_Rule WHERE ProductQuestionId = 2856;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Other);
    }

    #[test]
    fn infer_kind_ignores_keywords_inside_string_literals() {
        let sql = "\
SELECT 'UPDATE ProductQuestion_Rule SET EffectiveFrom = ''2026-05-01''' AS script_text;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Other);
    }

    #[test]
    fn infer_kind_for_select_then_update_is_dml() {
        let sql = "\
SELECT TOP 1 Id FROM ProductQuestion_Rule;
UPDATE ProductQuestion_Rule SET EffectiveFrom = '2026-05-01' WHERE ProductQuestionId = 2856;
";

        assert_eq!(infer_statement_kind(sql), StatementKind::Dml);
    }

    // --- parse_sql_statements: scope preservation + splitting ---

    #[test]
    fn splits_independent_selects_on_semicolons() {
        let sql = "SELECT 1; SELECT 2; SELECT 3;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 3);
        assert!(stmts[0].trim().ends_with(';'));
    }

    #[test]
    fn keeps_declare_batch_together_when_declare_leads() {
        // Old behavior: starts_with("DECLARE") kept the whole input together.
        // The new logic must preserve that.
        let sql = "DECLARE @x INT = 1; SELECT @x;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(
            stmts.len(),
            1,
            "DECLARE-led batch must stay as one statement"
        );
        assert!(stmts[0].contains("DECLARE @x"));
    }

    #[test]
    fn keeps_declare_batch_together_when_declare_is_in_the_middle() {
        // Regression: previously only `starts_with DECLARE/SET` triggered
        // scope preservation, so a non-DECLARE prefix caused the batch to be
        // split at every `;`, losing variable scope and producing spurious
        // (often errored) result tabs.
        let sql = "USE [MyDb];\nDECLARE @x INT = 1;\nSELECT @x;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(
            stmts.len(),
            1,
            "USE-prefixed DECLARE batch must stay as one"
        );
        assert!(stmts[0].contains("USE [MyDb]"));
        assert!(stmts[0].contains("DECLARE @x"));
        assert!(stmts[0].contains("SELECT @x"));
    }

    #[test]
    fn keeps_set_session_statements_with_following_select() {
        // `SET NOCOUNT ON; SELECT ...` — splitting would create an empty
        // result tab for the SET plus the real SELECT tab. Keeping the batch
        // together yields only the SELECT's rowset.
        let sql = "SET NOCOUNT ON;\nSELECT * FROM Users;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn keeps_temp_table_create_then_select_together() {
        // The user's reported scenario: temp table + SELECT INTO then SELECT
        // FROM it, split on `;`, errors on the second tab if the first failed.
        let sql = "SELECT * INTO #t FROM Foo;\nSELECT * FROM #t;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(
            stmts.len(),
            1,
            "temp-table batch must stay as one statement"
        );
        assert!(stmts[0].contains("#t"));
    }

    #[test]
    fn still_splits_across_GO_within_declare_batch() {
        // GO is a real batch separator in T-SQL — variables do not survive
        // across it. Even a DECLARE-containing batch splits on GO.
        let sql = "DECLARE @x INT = 1;\nSELECT @x;\nGO\nSELECT @x;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("DECLARE @x"));
        assert!(!stmts[1].contains("DECLARE"));
    }

    #[test]
    fn does_not_split_on_semicolons_inside_strings() {
        let sql = "SELECT 'a;b' AS x; SELECT 'c' AS y;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("'a;b'"));
    }

    #[test]
    fn does_not_split_on_GO_inside_block_comment() {
        let sql = "/* GO */\nSELECT 1;";
        let stmts = parse_sql_statements(sql);
        assert_eq!(stmts.len(), 1);
    }

    // --- batch_needs_scope_preservation: detection edge cases ---

    #[test]
    fn scope_detects_declare_anywhere_outside_comments() {
        assert!(batch_needs_scope_preservation(
            "SELECT 1; DECLARE @x INT; SELECT @x;"
        ));
    }

    #[test]
    fn scope_ignores_declare_inside_string_literal() {
        assert!(!batch_needs_scope_preservation(
            "SELECT 'DECLARE @x'; SELECT 1;"
        ));
    }

    #[test]
    fn scope_detects_hash_temp_table_reference() {
        assert!(batch_needs_scope_preservation(
            "SELECT * INTO #t FROM Foo; SELECT * FROM #t;"
        ));
        assert!(batch_needs_scope_preservation("SELECT * FROM ##global;"));
    }

    #[test]
    fn scope_ignores_hash_inside_bracketed_identifier() {
        // `[#count]` is a column alias, not a temp table.
        assert!(!batch_needs_scope_preservation(
            "SELECT [x] AS [#count] FROM T; SELECT 1;"
        ));
    }

    #[test]
    fn scope_ignores_hash_inside_line_comment() {
        assert!(!batch_needs_scope_preservation(
            "-- SELECT * FROM #temp\nSELECT 1;"
        ));
    }

    #[test]
    fn scope_detects_set_at_statement_start_only() {
        assert!(batch_needs_scope_preservation("SET @x = 1; SELECT @x;"));
        assert!(batch_needs_scope_preservation("SET NOCOUNT ON; SELECT 1;"));
        // SET inside UPDATE...SET is NOT a statement-start SET.
        assert!(!batch_needs_scope_preservation("UPDATE T SET x = 1;"));
    }

    #[test]
    fn scope_returns_false_for_plain_independent_selects() {
        assert!(!batch_needs_scope_preservation("SELECT 1; SELECT 2;"));
    }
}
