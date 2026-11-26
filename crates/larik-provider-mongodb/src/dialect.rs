use larik_data::sql_language::{
    IdentifierQuoteStyle, split_semicolon_statements_with_options, suggest_completions_with_snippets,
};
use larik_data::{
    CompletionCandidate, CompletionKind, CompletionRequest, CompletionResult, EditorLanguage,
    TextSpan,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementKind {
    Sql,
    Shell,
}

/// Classify a single statement as MongoDB shell (`db.<...>`) or T-SQL-on-MongoDB.
pub fn classify_statement(stmt: &str) -> StatementKind {
    let trimmed = strip_leading_comments(stmt).trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 3
        && bytes[0..2].eq_ignore_ascii_case(b"db")
        && matches!(bytes[2], b'.' | b'[')
    {
        StatementKind::Shell
    } else {
        StatementKind::Sql
    }
}

fn strip_leading_comments(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if s.starts_with("--") || s.starts_with("//") {
            match s.find('\n') {
                Some(nl) => s = &s[nl + 1..],
                None => return "",
            }
            continue;
        }
        if s.starts_with("/*") {
            match s[2..].find("*/") {
                Some(end) => s = &s[2 + end + 2..],
                None => return s,
            }
            continue;
        }
        return s;
    }
}

pub const MONGO_SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "JOIN", "INNER JOIN", "LEFT JOIN", "LEFT OUTER JOIN", "ON", "AS",
    "AND", "OR", "NOT", "IN", "BETWEEN", "LIKE", "IS NULL", "IS NOT NULL", "ORDER BY", "GROUP BY",
    "HAVING", "TOP", "DISTINCT", "LIMIT", "OFFSET", "FETCH FIRST", "ROWS ONLY", "INSERT INTO",
    "VALUES", "UPDATE", "SET", "DELETE FROM", "USE", "CASE", "WHEN", "THEN", "ELSE", "END", "ASC",
    "DESC", "NULL", "TRUE", "FALSE",
];

pub const MONGO_SQL_FUNCTIONS: &[&str] = &[
    "COUNT", "SUM", "AVG", "MIN", "MAX", "COALESCE", "ISNULL", "IFNULL", "UPPER", "LOWER", "LEN",
    "LENGTH", "TRIM", "LTRIM", "RTRIM", "CONCAT", "SUBSTRING", "SUBSTR", "ABS", "CEILING", "CEIL",
    "FLOOR", "ROUND", "SQRT", "POWER", "YEAR", "MONTH", "DAY", "HOUR", "MINUTE", "SECOND", "NOW",
    "GETDATE", "GETUTCDATE", "ObjectId", "ISODate", "UUID", "GUID", "NumberInt", "NumberLong",
    "NumberDecimal",
];

const SHELL_DB_METHODS: &[(&str, &str)] = &[
    ("getCollectionNames()", "List all collections in the database"),
    ("getCollection('')", "Reference a collection by string name"),
    ("stats()", "Return database storage statistics"),
];

const SHELL_COLLECTION_METHODS: &[(&str, &str)] = &[
    ("find({})", "Find documents matching a filter"),
    ("findOne({})", "Find the first matching document"),
    ("countDocuments({})", "Count documents matching a filter"),
    ("estimatedDocumentCount()", "Fast estimated document count"),
    ("distinct('')", "List distinct values for a field"),
    ("aggregate([])", "Run an aggregation pipeline"),
    ("getIndexes()", "List collection indexes"),
    ("stats()", "Return collection storage statistics"),
    ("insertOne({})", "Insert one document"),
    ("insertMany([{}])", "Insert multiple documents"),
    ("updateOne({}, { $set: {} })", "Update first matching document"),
    ("updateMany({}, { $set: {} })", "Update all matching documents"),
    ("deleteOne({})", "Delete first matching document"),
    ("deleteMany({})", "Delete all matching documents"),
];

#[derive(Debug, Default, Clone)]
pub struct MongoTSqlDialect;

impl MongoTSqlDialect {
    pub const fn new() -> Self {
        Self
    }
}

impl EditorLanguage for MongoTSqlDialect {
    fn syntax_id(&self) -> &'static str {
        "sql"
    }

    fn split_statements(&self, text: &str) -> Vec<TextSpan> {
        let mut out = Vec::new();
        split_semicolon_statements_with_options(text, 0, true, &mut out);
        out
    }

    fn split_batches(&self, text: &str) -> Vec<TextSpan> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let start = text.len() - text.trim_start().len();
        let end = start + trimmed.len();
        vec![TextSpan {
            start,
            end,
            text: trimmed.to_string(),
        }]
    }

    fn keywords(&self) -> &'static [&'static str] {
        MONGO_SQL_KEYWORDS
    }

    fn functions(&self) -> &'static [&'static str] {
        MONGO_SQL_FUNCTIONS
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        IdentifierQuoteStyle::Brackets.quote(identifier)
    }

    fn preview_statement(&self, _schema: Option<&str>, table: &str, limit: usize) -> String {
        format!("SELECT * FROM {} LIMIT {limit};", self.quote_identifier(table))
    }

    fn suggest_completions(&self, request: &CompletionRequest<'_>) -> CompletionResult {
        if let Some(shell_res) = try_shell_completions(request) {
            return shell_res;
        }
        let (replace_start, items) = suggest_completions_with_snippets(
            request.text,
            request.byte_offset,
            request.tables,
            request.databases,
            request.snippets,
            request.clipboard,
            self,
            request.is_explicit,
        );
        CompletionResult {
            replace_start,
            items,
        }
    }
}

/// Detect `db.<prefix>` (collections + DB methods) and `db.<collection>.<prefix>` (mongosh methods).
fn try_shell_completions(request: &CompletionRequest<'_>) -> Option<CompletionResult> {
    let caret = request.byte_offset.min(request.text.len());
    let before = &request.text[..caret];

    // Scan back to the start of the current dotted token chain (e.g. `db.orders.fi`).
    let mut chain_start = before.len();
    for (idx, ch) in before.char_indices().rev() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-' | '$') {
            chain_start = idx;
        } else {
            break;
        }
    }
    let chain = &before[chain_start..];
    let parts: Vec<&str> = chain.split('.').collect();
    if parts.len() < 2 || !parts[0].eq_ignore_ascii_case("db") {
        return None;
    }

    let typed = parts.last().copied().unwrap_or("");
    let typed_lower = typed.to_ascii_lowercase();
    let replace_start = caret - typed.len();

    let mut items = Vec::new();
    if parts.len() == 2 {
        // `db.<prefix>`: suggest collections first, then DB methods.
        for table in request.tables {
            if typed_lower.is_empty()
                || table.name.to_ascii_lowercase().contains(&typed_lower)
            {
                items.push(CompletionCandidate {
                    label: table.name.clone(),
                    detail: Some("Collection".to_string()),
                    insert_text: table.name.clone(),
                    filter_text: Some(table.name.clone()),
                    kind: CompletionKind::Table,
                    caret_offset: None,
                    selection: None,
                    documentation: None,
                });
            }
        }
        for &(method, desc) in SHELL_DB_METHODS {
            let name = method.split('(').next().unwrap_or(method);
            if typed_lower.is_empty() || name.to_ascii_lowercase().contains(&typed_lower) {
                items.push(CompletionCandidate {
                    label: method.to_string(),
                    detail: Some(desc.to_string()),
                    insert_text: method.to_string(),
                    filter_text: Some(name.to_string()),
                    kind: CompletionKind::Function,
                    caret_offset: None,
                    selection: None,
                    documentation: None,
                });
            }
        }
    } else {
        // `db.<collection>.<prefix>`: suggest mongosh collection methods.
        for &(method, desc) in SHELL_COLLECTION_METHODS {
            let name = method.split('(').next().unwrap_or(method);
            if typed_lower.is_empty() || name.to_ascii_lowercase().contains(&typed_lower) {
                items.push(CompletionCandidate {
                    label: method.to_string(),
                    detail: Some(desc.to_string()),
                    insert_text: method.to_string(),
                    filter_text: Some(name.to_string()),
                    kind: CompletionKind::Function,
                    caret_offset: None,
                    selection: None,
                    documentation: None,
                });
            }
        }
    }

    Some(CompletionResult {
        replace_start,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use larik_data::{TableKind, TableSchema};

    #[test]
    fn classifies_sql_vs_shell_statements() {
        assert_eq!(classify_statement("SELECT * FROM [users]"), StatementKind::Sql);
        assert_eq!(
            classify_statement("-- comment\ndb.users.find({})"),
            StatementKind::Shell
        );
        assert_eq!(
            classify_statement("// js comment\ndb['orders'].countDocuments({})"),
            StatementKind::Shell
        );
        assert_eq!(classify_statement("USE [shop]"), StatementKind::Sql);
    }

    #[test]
    fn splits_statements_ignoring_semicolons_in_strings_and_slash_comments() {
        let d = MongoTSqlDialect::new();
        let spans = d.split_statements(
            "// a;b comment\ndb.users.find({ name: 'a;b' });\nSELECT * FROM [orders];",
        );
        assert_eq!(spans.len(), 2);
        assert!(spans[0].text.contains("db.users.find"));
        assert_eq!(spans[1].text, "SELECT * FROM [orders]");
    }

    #[test]
    fn suggests_collections_and_methods_for_db_prefix() {
        let d = MongoTSqlDialect::new();
        let tables = vec![TableSchema {
            schema: String::new(),
            name: "orders".into(),
            kind: TableKind::Table,
            columns: Vec::new(),
            primary_key: vec!["_id".into()],
            foreign_keys: Vec::new(),
        }];

        let q1 = "db.ord";
        let r1 = d.suggest_completions(&CompletionRequest {
            text: q1,
            byte_offset: q1.len(),
            tables: &tables,
            databases: &[],
            is_explicit: false,
            snippets: &[],
            clipboard: "",
        });
        assert_eq!(r1.replace_start, 3);
        assert!(r1.items.iter().any(|i| i.label == "orders"));

        let q2 = "db.orders.fi";
        let r2 = d.suggest_completions(&CompletionRequest {
            text: q2,
            byte_offset: q2.len(),
            tables: &tables,
            databases: &[],
            is_explicit: false,
            snippets: &[],
            clipboard: "",
        });
        assert_eq!(r2.replace_start, 10);
        assert!(r2.items.iter().any(|i| i.label.starts_with("find(")));
        assert!(r2.items.iter().any(|i| i.label.starts_with("findOne(")));
    }
}
