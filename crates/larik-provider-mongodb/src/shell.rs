use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures_util::TryStreamExt;
use larik_data::{QueryColumn, QueryResult};
use mongodb::Database;
use mongodb::bson::{Bson, Document, doc};

use crate::formatter::{
    bson_type_name, format_cell, parse_mongosh_document, parse_mongosh_document_array,
    parse_mongosh_value,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSuffix {
    pub method: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCall {
    pub collection: Option<String>,
    pub method: String,
    pub arguments: String,
    pub suffixes: Vec<ShellSuffix>,
}

pub fn end_of_string(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let quote = *bytes.get(start)?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let mut i = start + 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if b == quote {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn skip_ws_and_comments(text: &str, mut pos: usize) -> Result<usize> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    loop {
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos + 1 < len
            && ((bytes[pos] == b'/' && bytes[pos + 1] == b'/')
                || (bytes[pos] == b'-' && bytes[pos + 1] == b'-'))
        {
            pos += 2;
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
            pos += 2;
            let mut closed = false;
            while pos + 1 < len {
                if bytes[pos] == b'*' && bytes[pos + 1] == b'/' {
                    pos += 2;
                    closed = true;
                    break;
                }
                pos += 1;
            }
            if !closed {
                bail!("Unterminated block comment in MongoDB shell script.");
            }
            continue;
        }
        break;
    }
    Ok(pos)
}

fn find_matching_paren(text: &str, open_paren: usize) -> Result<usize> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    if bytes.get(open_paren) != Some(&b'(') {
        bail!("Expected '(' in MongoDB shell call.");
    }
    let mut depth = 1usize;
    let mut i = open_paren + 1;
    while i < len {
        let b = bytes[i];
        if b == b'\'' || b == b'"' {
            let Some(close) = end_of_string(text, i) else {
                bail!("Unterminated string literal in MongoDB shell call.");
            };
            i = close + 1;
            continue;
        }
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut closed = false;
            while i + 1 < len {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                bail!("Unterminated block comment in MongoDB shell call.");
            }
            continue;
        }
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                return Ok(i);
            }
        }
        i += 1;
    }
    bail!("Unterminated '(' in MongoDB shell call.")
}

pub fn split_top_level_args(args: &str) -> Result<Vec<String>> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;

    while i < len {
        let b = bytes[i];
        if b == b'\'' || b == b'"' {
            let Some(close) = end_of_string(trimmed, i) else {
                bail!("Unterminated string literal in arguments.");
            };
            i = close + 1;
            continue;
        }
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut closed = false;
            while i + 1 < len {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                bail!("Unterminated block comment in arguments.");
            }
            continue;
        }
        match b {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b',' if paren == 0 && brace == 0 && bracket == 0 => {
                out.push(trimmed[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(trimmed[start..].trim().to_string());
    Ok(out)
}

pub fn parse_shell_call(stmt: &str) -> Result<ShellCall> {
    let bytes = stmt.as_bytes();
    let len = bytes.len();
    let mut pos = skip_ws_and_comments(stmt, 0)?;

    if pos + 2 > len || !stmt[pos..pos + 2].eq_ignore_ascii_case("db") {
        bail!("MongoDB shell statement must start with 'db.'.");
    }
    pos += 2;
    pos = skip_ws_and_comments(stmt, pos)?;

    let mut collection: Option<String> = None;

    if pos < len && bytes[pos] == b'[' {
        pos += 1;
        pos = skip_ws_and_comments(stmt, pos)?;
        let Some(close_str) = end_of_string(stmt, pos) else {
            bail!("Expected quoted collection name inside db[...].");
        };
        let coll = match parse_mongosh_value(&stmt[pos..=close_str])? {
            Bson::String(s) if !s.is_empty() => s,
            _ => bail!("Collection name inside db[...] must be a non-empty string."),
        };
        pos = skip_ws_and_comments(stmt, close_str + 1)?;
        if bytes.get(pos) != Some(&b']') {
            bail!("Expected ']' after collection name.");
        }
        pos = skip_ws_and_comments(stmt, pos + 1)?;
        if bytes.get(pos) != Some(&b'.') {
            bail!("Expected '.' after db['{coll}'].");
        }
        pos += 1;
        collection = Some(coll);
    } else if bytes.get(pos) == Some(&b'.') {
        pos += 1;
    } else {
        bail!("Expected '.' or '[' after 'db'.");
    }

    let first_ident_start = skip_ws_and_comments(stmt, pos)?;
    pos = first_ident_start;
    while pos < len && (bytes[pos].is_ascii_alphanumeric() || matches!(bytes[pos], b'_' | b'-' | b'$')) {
        pos += 1;
    }
    let first_ident = stmt[first_ident_start..pos].to_string();
    if first_ident.is_empty() {
        bail!("Expected collection or method name after 'db.'.");
    }
    pos = skip_ws_and_comments(stmt, pos)?;

    let (method, open_paren) = if collection.is_none()
        && first_ident.eq_ignore_ascii_case("getCollection")
        && bytes.get(pos) == Some(&b'(')
    {
        let close = find_matching_paren(stmt, pos)?;
        let arg_str = stmt[pos + 1..close].trim();
        let coll = match parse_mongosh_value(arg_str)? {
            Bson::String(s) if !s.is_empty() => s,
            _ => bail!("db.getCollection(...) expects a non-empty string collection name."),
        };
        collection = Some(coll);
        pos = skip_ws_and_comments(stmt, close + 1)?;
        if bytes.get(pos) != Some(&b'.') {
            bail!("Expected '.<method>(...)' after db.getCollection(...).");
        }
        pos = skip_ws_and_comments(stmt, pos + 1)?;
        let m_start = pos;
        while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
            pos += 1;
        }
        let m = stmt[m_start..pos].to_string();
        pos = skip_ws_and_comments(stmt, pos)?;
        if bytes.get(pos) != Some(&b'(') {
            bail!("Expected '(' after method '{m}'.");
        }
        (m, pos)
    } else if bytes.get(pos) == Some(&b'(') {
        (first_ident, pos)
    } else if collection.is_none() && bytes.get(pos) == Some(&b'.') {
        let mut segments = vec![first_ident];
        loop {
            pos = skip_ws_and_comments(stmt, pos + 1)?;
            let seg_start = pos;
            while pos < len
                && (bytes[pos].is_ascii_alphanumeric() || matches!(bytes[pos], b'_' | b'-' | b'$'))
            {
                pos += 1;
            }
            let seg = stmt[seg_start..pos].to_string();
            if seg.is_empty() {
                bail!("Expected identifier after '.' in MongoDB shell call.");
            }
            segments.push(seg);
            pos = skip_ws_and_comments(stmt, pos)?;
            if bytes.get(pos) == Some(&b'.') {
                continue;
            }
            if bytes.get(pos) == Some(&b'(') {
                break;
            }
            bail!("Expected '(' or '.' in MongoDB shell call.");
        }
        let m = segments.pop().unwrap();
        collection = Some(segments.join("."));
        (m, pos)
    } else {
        bail!("Expected '(' or '.' after '{first_ident}' in MongoDB shell call.");
    };

    let close_paren = find_matching_paren(stmt, open_paren)?;
    let arguments = stmt[open_paren + 1..close_paren].to_string();
    pos = skip_ws_and_comments(stmt, close_paren + 1)?;

    let mut suffixes = Vec::new();
    while pos < len && bytes[pos] == b'.' {
        pos = skip_ws_and_comments(stmt, pos + 1)?;
        let s_start = pos;
        while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
            pos += 1;
        }
        let s_method = stmt[s_start..pos].to_string();
        if s_method.is_empty() {
            bail!("Expected chained method name after '.'.");
        }
        pos = skip_ws_and_comments(stmt, pos)?;
        if bytes.get(pos) != Some(&b'(') {
            bail!("Expected '(' after chained method '{s_method}'.");
        }
        let s_close = find_matching_paren(stmt, pos)?;
        let s_body = stmt[pos + 1..s_close].to_string();
        suffixes.push(ShellSuffix {
            method: s_method,
            body: s_body,
        });
        pos = skip_ws_and_comments(stmt, s_close + 1)?;
    }

    while pos < len && bytes[pos] == b';' {
        pos = skip_ws_and_comments(stmt, pos + 1)?;
    }
    if pos < len {
        bail!(
            "Unexpected trailing tokens '{}' in MongoDB shell call.",
            &stmt[pos..]
        );
    }

    Ok(ShellCall {
        collection,
        method,
        arguments,
        suffixes,
    })
}

pub fn documents_to_query_result(
    docs: &[Document],
    explicit_columns: &[String],
    max_rows: usize,
    is_truncated: bool,
    elapsed: Duration,
    database: Option<String>,
    statement: Option<String>,
) -> QueryResult {
    let col_names: Vec<String> = if !explicit_columns.is_empty() {
        explicit_columns.to_vec()
    } else {
        let mut ordered = Vec::new();
        let mut seen = BTreeSet::new();
        if docs.iter().any(|d| d.contains_key("_id")) {
            ordered.push("_id".to_string());
            seen.insert("_id".to_string());
        }
        for doc in docs {
            for k in doc.keys() {
                if seen.insert(k.clone()) {
                    ordered.push(k.clone());
                }
            }
        }
        ordered
    };

    let mut columns = Vec::with_capacity(col_names.len());
    for name in &col_names {
        let inferred = docs
            .iter()
            .find_map(|d| d.get(name).filter(|v| !matches!(v, Bson::Null | Bson::Undefined)))
            .map(|v| bson_type_name(v).to_string());
        columns.push(QueryColumn {
            name: name.clone(),
            data_type: inferred,
        });
    }

    let mut rows = Vec::with_capacity(docs.len());
    for doc in docs {
        let mut row = Vec::with_capacity(col_names.len());
        for name in &col_names {
            let cell = doc.get(name).and_then(format_cell);
            row.push(cell);
        }
        rows.push(row);
    }

    let row_count = rows.len();
    let message = if is_truncated {
        format!("Showing first {max_rows} documents (truncated).")
    } else if row_count == 1 {
        "1 document returned.".to_string()
    } else {
        format!("{row_count} documents returned.")
    };

    QueryResult {
        columns,
        rows,
        records_affected: None,
        message,
        is_error: false,
        elapsed,
        database,
        is_truncated,
        row_limit: is_truncated.then_some(max_rows),
        messages: Vec::new(),
        statement,
    }
}

pub async fn execute_shell_statement(
    db: &Database,
    raw_stmt: &str,
    max_rows: usize,
    read_only: bool,
) -> QueryResult {
    let start = Instant::now();
    let db_name = Some(db.name().to_string());

    let call = match parse_shell_call(raw_stmt) {
        Ok(c) => c,
        Err(err) => {
            let mut r = QueryResult::error(err.to_string(), start.elapsed(), db_name);
            r.statement = Some(raw_stmt.to_string());
            return r;
        }
    };

    if read_only && !crate::read_only::is_proven_read_shell_call(&call) {
        let mut r = QueryResult::error(
            "Statement refused: this workspace is in read-only mode.",
            start.elapsed(),
            db_name,
        );
        r.statement = Some(raw_stmt.to_string());
        return r;
    }

    match run_shell_call(db, &call, raw_stmt, max_rows, start).await {
        Ok(res) => res,
        Err(err) => {
            let mut r = QueryResult::error(err.to_string(), start.elapsed(), db_name);
            r.statement = Some(raw_stmt.to_string());
            r
        }
    }
}

async fn run_shell_call(
    db: &Database,
    call: &ShellCall,
    raw_stmt: &str,
    max_rows: usize,
    start: Instant,
) -> Result<QueryResult> {
    let db_name = Some(db.name().to_string());
    let args = split_top_level_args(&call.arguments)?;

    let Some(ref coll_name) = call.collection else {
        if !call.suffixes.is_empty() {
            bail!(
                "Chained cursor modifiers are not supported on db.{}().",
                call.method
            );
        }
        return match call.method.to_ascii_lowercase().as_str() {
            "getcollectionnames" => {
                let mut names = db
                    .list_collection_names()
                    .await
                    .context("Failed to list collections")?;
                names.sort();
                let docs: Vec<Document> = names
                    .into_iter()
                    .map(|n| doc! { "collection": n })
                    .collect();
                Ok(documents_to_query_result(
                    &docs,
                    &["collection".to_string()],
                    max_rows,
                    false,
                    start.elapsed(),
                    db_name,
                    Some(raw_stmt.to_string()),
                ))
            }
            "stats" => {
                let stats = db
                    .run_command(doc! { "dbStats": 1 })
                    .await
                    .context("db.stats() failed")?;
                Ok(documents_to_query_result(
                    &[stats],
                    &[],
                    max_rows,
                    false,
                    start.elapsed(),
                    db_name,
                    Some(raw_stmt.to_string()),
                ))
            }
            other => bail!(
                "Unsupported MongoDB database method 'db.{other}()'. Supported: getCollectionNames(), stats()."
            ),
        };
    };

    let coll = db.collection::<Document>(coll_name);
    let method_lower = call.method.to_ascii_lowercase();

    if method_lower != "find" && !call.suffixes.is_empty() {
        bail!(
            "Chained cursor modifiers are only supported on find(), not {}().",
            call.method
        );
    }

    match method_lower.as_str() {
        "find" => {
            let filter = args
                .first()
                .map(|s| parse_mongosh_document(s))
                .transpose()?
                .unwrap_or_default();
            let mut projection = args
                .get(1)
                .map(|s| parse_mongosh_document(s))
                .transpose()?;
            let mut sort_doc: Option<Document> = None;
            let mut skip_val: Option<u64> = None;
            let mut user_limit: Option<i64> = None;

            for suffix in &call.suffixes {
                match suffix.method.to_ascii_lowercase().as_str() {
                    "sort" => {
                        sort_doc = Some(parse_mongosh_document(&suffix.body)?);
                    }
                    "project" => {
                        projection = Some(parse_mongosh_document(&suffix.body)?);
                    }
                    "skip" => {
                        let n: i64 = suffix
                            .body
                            .trim()
                            .parse()
                            .with_context(|| format!("Invalid .skip({}) value.", suffix.body))?;
                        if n < 0 {
                            bail!(".skip() expects a non-negative integer.");
                        }
                        skip_val = Some(n as u64);
                    }
                    "limit" => {
                        let n: i64 = suffix
                            .body
                            .trim()
                            .parse()
                            .with_context(|| format!("Invalid .limit({}) value.", suffix.body))?;
                        if n < 0 {
                            bail!(".limit() expects a non-negative integer.");
                        }
                        user_limit = Some(n);
                    }
                    other => bail!(
                        "Unsupported find() cursor modifier '.{other}()'. Supported: .sort(), .skip(), .limit(), .project()."
                    ),
                }
            }

            let effective_cap = match user_limit {
                Some(l) if (l as usize) <= max_rows => l,
                _ => (max_rows as i64).saturating_add(1),
            };

            let mut action = coll.find(filter).limit(effective_cap);
            if let Some(proj) = projection {
                action = action.projection(proj);
            }
            if let Some(sort) = sort_doc {
                action = action.sort(sort);
            }
            if let Some(skip) = skip_val {
                action = action.skip(skip);
            }

            let mut cursor = action.await.context("find() failed")?;
            let mut docs = Vec::new();
            let mut truncated = false;
            while let Some(doc) = cursor.try_next().await.context("Reading find() cursor failed")? {
                if docs.len() < max_rows {
                    docs.push(doc);
                } else {
                    truncated = true;
                    break;
                }
            }

            Ok(documents_to_query_result(
                &docs,
                &[],
                max_rows,
                truncated,
                start.elapsed(),
                db_name,
                Some(raw_stmt.to_string()),
            ))
        }
        "findone" => {
            let filter = args
                .first()
                .map(|s| parse_mongosh_document(s))
                .transpose()?
                .unwrap_or_default();
            let projection = args
                .get(1)
                .map(|s| parse_mongosh_document(s))
                .transpose()?;
            let mut action = coll.find_one(filter);
            if let Some(proj) = projection {
                action = action.projection(proj);
            }
            let doc_opt = action.await.context("findOne() failed")?;
            let docs: Vec<Document> = doc_opt.into_iter().collect();
            Ok(documents_to_query_result(
                &docs,
                &[],
                max_rows,
                false,
                start.elapsed(),
                db_name,
                Some(raw_stmt.to_string()),
            ))
        }
        "count" | "countdocuments" => {
            let filter = args
                .first()
                .map(|s| parse_mongosh_document(s))
                .transpose()?
                .unwrap_or_default();
            let count = coll
                .count_documents(filter)
                .await
                .context("countDocuments() failed")?;
            let docs = vec![doc! { "count": count as i64 }];
            Ok(documents_to_query_result(
                &docs,
                &["count".to_string()],
                max_rows,
                false,
                start.elapsed(),
                db_name,
                Some(raw_stmt.to_string()),
            ))
        }
        "estimateddocumentcount" => {
            let count = coll
                .estimated_document_count()
                .await
                .context("estimatedDocumentCount() failed")?;
            let docs = vec![doc! { "count": count as i64 }];
            Ok(documents_to_query_result(
                &docs,
                &["count".to_string()],
                max_rows,
                false,
                start.elapsed(),
                db_name,
                Some(raw_stmt.to_string()),
            ))
        }
        "distinct" => {
            let Some(field_arg) = args.first() else {
                bail!("distinct(field, [filter]) expects a field name.");
            };
            let field = match parse_mongosh_value(field_arg)? {
                Bson::String(s) => s,
                other => bail!("distinct() field name must be a string, got {other}."),
            };
            let filter = args
                .get(1)
                .map(|s| parse_mongosh_document(s))
                .transpose()?
                .unwrap_or_default();
            let vals = coll
                .distinct(&field, filter)
                .await
                .context("distinct() failed")?;
            let truncated = vals.len() > max_rows;
            let docs: Vec<Document> = vals
                .into_iter()
                .take(max_rows)
                .map(|v| doc! { &field: v })
                .collect();
            Ok(documents_to_query_result(
                &docs,
                &[field],
                max_rows,
                truncated,
                start.elapsed(),
                db_name,
                Some(raw_stmt.to_string()),
            ))
        }
        "aggregate" => {
            let Some(pipe_arg) = args.first() else {
                bail!("aggregate([...]) expects an aggregation pipeline array.");
            };
            let mut pipeline = parse_mongosh_document_array(pipe_arg)?;
            let has_write = crate::compiler::pipeline_has_write_stage(&pipeline);
            if !has_write {
                pipeline.push(doc! { "$limit": (max_rows as i64).saturating_add(1) });
            }
            let mut cursor = coll
                .aggregate(pipeline)
                .await
                .context("aggregate() failed")?;
            let mut docs = Vec::new();
            let mut truncated = false;
            while let Some(doc) = cursor
                .try_next()
                .await
                .context("Reading aggregate() cursor failed")?
            {
                if docs.len() < max_rows {
                    docs.push(doc);
                } else {
                    truncated = true;
                    break;
                }
            }
            Ok(documents_to_query_result(
                &docs,
                &[],
                max_rows,
                truncated,
                start.elapsed(),
                db_name,
                Some(raw_stmt.to_string()),
            ))
        }
        "getindexes" => {
            let mut cursor = coll
                .list_indexes()
                .await
                .context("getIndexes() failed")?;
            let mut docs = Vec::new();
            while let Some(idx) = cursor
                .try_next()
                .await
                .context("Reading index list failed")?
            {
                let name = idx
                    .options
                    .as_ref()
                    .and_then(|o| o.name.clone())
                    .unwrap_or_default();
                let unique = idx
                    .options
                    .as_ref()
                    .and_then(|o| o.unique)
                    .unwrap_or(false);
                docs.push(doc! {
                    "name": name,
                    "key": idx.keys,
                    "unique": unique,
                });
            }
            Ok(documents_to_query_result(
                &docs,
                &["name".to_string(), "key".to_string(), "unique".to_string()],
                max_rows,
                false,
                start.elapsed(),
                db_name,
                Some(raw_stmt.to_string()),
            ))
        }
        "stats" => {
            let stats = db
                .run_command(doc! { "collStats": coll_name })
                .await
                .context("collStats failed")?;
            Ok(documents_to_query_result(
                &[stats],
                &[],
                max_rows,
                false,
                start.elapsed(),
                db_name,
                Some(raw_stmt.to_string()),
            ))
        }
        "insertone" => {
            let Some(doc_arg) = args.first() else {
                bail!("insertOne(doc) expects a document argument.");
            };
            let doc = parse_mongosh_document(doc_arg)?;
            let res = coll
                .insert_one(doc)
                .await
                .context("insertOne() failed")?;
            let id_str = format_cell(&res.inserted_id).unwrap_or_default();
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: Some(1),
                message: format!("1 document inserted (_id: {id_str})."),
                is_error: false,
                elapsed: start.elapsed(),
                database: db_name,
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: Some(raw_stmt.to_string()),
            })
        }
        "insertmany" => {
            let Some(arr_arg) = args.first() else {
                bail!("insertMany([...]) expects an array of documents.");
            };
            let docs = parse_mongosh_document_array(arr_arg)?;
            let res = coll
                .insert_many(docs)
                .await
                .context("insertMany() failed")?;
            let count = res.inserted_ids.len() as i64;
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: Some(count),
                message: format!("{count} document(s) inserted."),
                is_error: false,
                elapsed: start.elapsed(),
                database: db_name,
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: Some(raw_stmt.to_string()),
            })
        }
        "updateone" => {
            if args.len() < 2 {
                bail!("updateOne(filter, update) expects 2 arguments.");
            }
            let filter = parse_mongosh_document(&args[0])?;
            let update = parse_mongosh_document(&args[1])?;
            let res = coll
                .update_one(filter, update)
                .await
                .context("updateOne() failed")?;
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: Some(res.modified_count as i64),
                message: format!(
                    "Matched {} document(s), modified {} document(s).",
                    res.matched_count, res.modified_count
                ),
                is_error: false,
                elapsed: start.elapsed(),
                database: db_name,
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: Some(raw_stmt.to_string()),
            })
        }
        "updatemany" => {
            if args.len() < 2 {
                bail!("updateMany(filter, update) expects 2 arguments.");
            }
            let filter = parse_mongosh_document(&args[0])?;
            let update = parse_mongosh_document(&args[1])?;
            let res = coll
                .update_many(filter, update)
                .await
                .context("updateMany() failed")?;
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: Some(res.modified_count as i64),
                message: format!(
                    "Matched {} document(s), modified {} document(s).",
                    res.matched_count, res.modified_count
                ),
                is_error: false,
                elapsed: start.elapsed(),
                database: db_name,
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: Some(raw_stmt.to_string()),
            })
        }
        "deleteone" => {
            let filter = args
                .first()
                .map(|s| parse_mongosh_document(s))
                .transpose()?
                .unwrap_or_default();
            let res = coll
                .delete_one(filter)
                .await
                .context("deleteOne() failed")?;
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: Some(res.deleted_count as i64),
                message: format!("{} document(s) deleted.", res.deleted_count),
                is_error: false,
                elapsed: start.elapsed(),
                database: db_name,
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: Some(raw_stmt.to_string()),
            })
        }
        "deletemany" => {
            let filter = args
                .first()
                .map(|s| parse_mongosh_document(s))
                .transpose()?
                .unwrap_or_default();
            let res = coll
                .delete_many(filter)
                .await
                .context("deleteMany() failed")?;
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                records_affected: Some(res.deleted_count as i64),
                message: format!("{} document(s) deleted.", res.deleted_count),
                is_error: false,
                elapsed: start.elapsed(),
                database: db_name,
                is_truncated: false,
                row_limit: None,
                messages: Vec::new(),
                statement: Some(raw_stmt.to_string()),
            })
        }
        other => bail!(
            "Unsupported MongoDB collection method '.{other}()'. Supported: find, findOne, count, countDocuments, estimatedDocumentCount, distinct, aggregate, getIndexes, stats, insertOne, insertMany, updateOne, updateMany, deleteOne, deleteMany."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_collection_and_database_shell_calls() {
        let c1 = parse_shell_call("db.users.find({ status: 'open' }).sort({ age: -1 }).limit(5)")
            .unwrap();
        assert_eq!(c1.collection.as_deref(), Some("users"));
        assert_eq!(c1.method, "find");
        assert_eq!(c1.suffixes.len(), 2);
        assert_eq!(c1.suffixes[0].method, "sort");
        assert_eq!(c1.suffixes[1].method, "limit");
        assert_eq!(c1.suffixes[1].body.trim(), "5");

        let c2 = parse_shell_call("db.getCollectionNames()").unwrap();
        assert_eq!(c2.collection, None);
        assert_eq!(c2.method, "getCollectionNames");

        let c3 = parse_shell_call("db.getCollection('order.items').findOne({ _id: 1 })").unwrap();
        assert_eq!(c3.collection.as_deref(), Some("order.items"));
        assert_eq!(c3.method, "findOne");

        let c4 = parse_shell_call("db['my-coll'].countDocuments({})").unwrap();
        assert_eq!(c4.collection.as_deref(), Some("my-coll"));
        assert_eq!(c4.method, "countDocuments");
    }

    #[test]
    fn splits_top_level_args_respecting_nested_braces_and_strings() {
        let args = split_top_level_args("{ a: 1, b: 'x,y' }, { name: 1, _id: 0 }").unwrap();
        assert_eq!(args, vec!["{ a: 1, b: 'x,y' }", "{ name: 1, _id: 0 }"]);
    }
}
