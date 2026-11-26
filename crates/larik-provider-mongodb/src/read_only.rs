use larik_data::EditorLanguage;
use larik_data::read_only_sql::ReadOnlyStatementPolicy;

use crate::commands::MongoCommand;
use crate::compiler::{TSqlMongoCompiler, pipeline_has_write_stage};
use crate::dialect::{MongoTSqlDialect, StatementKind, classify_statement};
use crate::shell::{ShellCall, end_of_string, parse_shell_call};

const DB_READ_METHODS: &[&str] = &["getCollectionNames", "stats"];
const COLLECTION_READ_METHODS: &[&str] = &[
    "find",
    "findOne",
    "count",
    "countDocuments",
    "estimatedDocumentCount",
    "distinct",
    "getIndexes",
    "stats",
];
const READ_SUFFIXES: &[&str] = &["limit", "skip", "sort", "project"];
const WRITE_STAGES: &[&str] = &["$out", "$merge"];

/// Fail-closed static read-only classifier for MongoDB (matching `Larik.Data.MongoDB.MongoReadOnlyPolicy`).
#[derive(Debug, Default, Clone, Copy)]
pub struct MongoReadOnlyPolicy;

impl ReadOnlyStatementPolicy for MongoReadOnlyPolicy {
    fn is_proven_read(&self, sql: &str) -> bool {
        if sql.trim().is_empty() {
            return false;
        }
        let dialect = MongoTSqlDialect::new();
        let statements = dialect.split_statements(sql);
        if statements.is_empty() {
            return false;
        }

        let mut units = 0usize;
        for span in statements {
            let trimmed = strip_comments_only(&span.text);
            if trimmed.is_empty() {
                continue;
            }
            units += 1;

            match classify_statement(&span.text) {
                StatementKind::Shell => {
                    let Ok(call) = parse_shell_call(&span.text) else {
                        return false;
                    };
                    if !is_proven_read_shell_call(&call) {
                        return false;
                    }
                }
                StatementKind::Sql => {
                    let Ok(cmd) = TSqlMongoCompiler::compile(&span.text) else {
                        return false;
                    };
                    match cmd {
                        MongoCommand::Select { pipeline, .. } => {
                            if pipeline_has_write_stage(&pipeline) {
                                return false;
                            }
                        }
                        MongoCommand::Use { .. } => {}
                        _ => return false,
                    }
                }
            }
        }

        units > 0
    }
}

pub(crate) fn is_proven_read_shell_call(call: &ShellCall) -> bool {
    let allowed_method = match call.collection {
        None => DB_READ_METHODS
            .iter()
            .any(|m| m.eq_ignore_ascii_case(&call.method)),
        Some(_) => COLLECTION_READ_METHODS
            .iter()
            .any(|m| m.eq_ignore_ascii_case(&call.method)),
    };
    if !allowed_method {
        return false;
    }

    if contains_write_stage(&call.arguments) {
        return false;
    }

    if call.suffixes.is_empty() {
        return true;
    }

    // Cursor modifiers are only valid on collection-level `find()`.
    if call.collection.is_none() || !call.method.eq_ignore_ascii_case("find") {
        return false;
    }

    for suffix in &call.suffixes {
        if !READ_SUFFIXES
            .iter()
            .any(|m| m.eq_ignore_ascii_case(&suffix.method))
        {
            return false;
        }
        if contains_write_stage(&suffix.body) {
            return false;
        }
        if suffix.method.eq_ignore_ascii_case("limit")
            || suffix.method.eq_ignore_ascii_case("skip")
        {
            let Ok(count) = suffix.body.trim().parse::<i64>() else {
                return false;
            };
            if count < 0 {
                return false;
            }
        }
    }

    true
}

/// Fail-closed check for `$out` / `$merge` keys (quoted or bare) in shell arguments.
/// Returns `true` if a write stage key is found OR if any string literal is unterminated.
fn contains_write_stage(arguments: &str) -> bool {
    let bytes = arguments.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            let Some(close) = end_of_string(arguments, i) else {
                // Fail closed on unterminated string literal.
                return true;
            };
            let key = &arguments[i + 1..close];
            if is_write_stage(key) && next_non_space(bytes, close + 1) == Some(b':') {
                return true;
            }
            i = close + 1;
            continue;
        }

        if b == b'$' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'$')) {
                i += 1;
            }
            let key = &arguments[start..i];
            if is_write_stage(key) && next_non_space(bytes, i) == Some(b':') {
                return true;
            }
            continue;
        }

        i += 1;
    }

    false
}

fn is_write_stage(name: &str) -> bool {
    WRITE_STAGES.contains(&name)
}

fn next_non_space(bytes: &[u8], mut idx: usize) -> Option<u8> {
    while idx < bytes.len() {
        if !bytes[idx].is_ascii_whitespace() {
            return Some(bytes[idx]);
        }
        idx += 1;
    }
    None
}

fn strip_comments_only(mut s: &str) -> &str {
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
        return s.trim();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proven_reads_are_allowed() {
        let policy = MongoReadOnlyPolicy;
        let allowed = [
            "SELECT * FROM [users] WHERE [age] > 18",
            "SELECT TOP 10 [name] FROM [users];",
            "SELECT * FROM [users]; USE [shop]",
            "USE [shop]",
            "-- the open orders\nSELECT * FROM [orders] WHERE [status] = 'open'",
            "db.users.find({})",
            "db.users.find({}, { name: 1 })",
            "db.users.find({}).sort({ age: -1 }).limit(5)",
            "db.users.find({}).skip(10).limit(5)",
            "db.users.find({}).project({ name: 1 })",
            "SELECT [city], COUNT(*) AS [n] FROM [users] GROUP BY [city] ORDER BY [n] DESC",
            "db.users.findOne({ _id: 1 })",
            "db.users.find({ name: 'a;b' });",
            "db.orders.count({ status: 'open' })",
            "db.orders.countDocuments({ status: 'open' })",
            "db.orders.estimatedDocumentCount()",
            "db.users.distinct('city')",
            "db.users.getIndexes()",
            "db.users.stats()",
            "db.getCollectionNames()",
            "db.stats()",
        ];

        for stmt in allowed {
            assert!(
                policy.is_proven_read(stmt),
                "expected read-only policy to allow: {stmt}"
            );
        }
    }

    #[test]
    fn writes_aggregates_and_malformed_statements_are_refused() {
        let policy = MongoReadOnlyPolicy;
        let refused = [
            "INSERT INTO [users] ([name]) VALUES ('x')",
            "UPDATE [users] SET [name] = 'x' WHERE [id] = 1",
            "DELETE FROM [users] WHERE [id] = 1",
            "DROP TABLE [users]",
            "db.users.insertOne({ a: 1 })",
            "db.users.insertMany([{ a: 1 }])",
            "db.users.updateMany({}, { $set: { a: 1 } })",
            "db.users.deleteMany({})",
            "db.runCommand({ dropDatabase: 1 })",
            "db.getSiblingDB('admin').adminCommand({ shutdown: 1 })",
            // Level mismatches
            "db.getIndexes()",
            "db.users.getCollectionNames()",
            "db.find({})",
            // aggregate is refused outright in read-only mode
            "db.orders.aggregate([{ $match: { status: 'open' } }, { $limit: 10 }])",
            "db.users.aggregate([{ $match: {} }, { $out: 'users' }])",
            "db.users.aggregate([{ $merge: { into: 'archive' } }])",
            "db.users.aggregate([{ \"$out\": \"users\" }])",
            "db.users.aggregate([{ \"\\u0024out\": \"users\" }])",
            "db.users.aggregate([{ $facet: { a: [{ $out: 'x' }] } }])",
            "db.users.find({}, { $out: 1 })",
            // Invalid cursor modifiers
            "db.users.find({}).forEach(function (u) { db.users.deleteOne({ _id: u._id }); })",
            "db.users.find({}).deleteMany({})",
            "db.users.count({}).limit(5)",
            "db.users.find({}).limit(-1)",
            // Poisoned batches
            "SELECT * FROM [users]; db.users.deleteMany({})",
            "db.users.find({}); db.users.insertOne({ a: 1 })",
            // Malformed / empty
            "select * from",
            "",
            "   ",
            "-- only a comment",
        ];

        for stmt in refused {
            assert!(
                !policy.is_proven_read(stmt),
                "expected read-only policy to REFUSE: {stmt}"
            );
        }
    }
}
