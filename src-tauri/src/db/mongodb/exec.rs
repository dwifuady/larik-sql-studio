// Executes parsed shell statements against a MongoDB client.

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::{FindOneOptions, FindOptions};
use mongodb::{Client, Collection, Cursor, Database};
use std::time::Duration;

use super::convert::{to_document, type_name};
use super::shell::{MethodCall, ShowTarget, Statement, Value};
use crate::db::driver::{ExecutionOutput, ResultSet};
use crate::db::error::ConnectionError;
use crate::db::query::ColumnInfo;

fn query_error(msg: impl Into<String>) -> ConnectionError {
    ConnectionError::QueryError(msg.into())
}

/// Collection methods the editor supports (also used for autocomplete docs)
pub const COLLECTION_METHODS: &[&str] = &[
    "find",
    "findOne",
    "aggregate",
    "countDocuments",
    "estimatedDocumentCount",
    "count",
    "distinct",
    "insertOne",
    "insertMany",
    "updateOne",
    "updateMany",
    "replaceOne",
    "deleteOne",
    "deleteMany",
    "getIndexes",
];

// ============================================================================
// Argument helpers
// ============================================================================

fn check_arity(call: &MethodCall, max: usize) -> Result<(), ConnectionError> {
    if call.args.len() > max {
        return Err(query_error(format!(
            "{}() accepts at most {max} argument{}",
            call.name,
            if max == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

/// Optional object argument (missing = empty document)
fn optional_doc(call: &MethodCall, index: usize, what: &str) -> Result<Document, ConnectionError> {
    match call.args.get(index) {
        None => Ok(Document::new()),
        Some(value) => to_document(value, what).map_err(query_error),
    }
}

fn required_doc(call: &MethodCall, index: usize, what: &str) -> Result<Document, ConnectionError> {
    match call.args.get(index) {
        None => Err(query_error(format!("{}() requires {what}", call.name))),
        Some(value) => to_document(value, what).map_err(query_error),
    }
}

fn doc_list(value: &Value, what: &str) -> Result<Vec<Document>, ConnectionError> {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| to_document(item, what).map_err(query_error))
            .collect(),
        _ => Err(query_error(format!("{what} must be an array of objects"))),
    }
}

fn non_negative(call: &MethodCall) -> Result<u64, ConnectionError> {
    match call.args.as_slice() {
        [Value::Int(n)] if *n >= 0 => Ok(*n as u64),
        [Value::Double(n)] if *n >= 0.0 && n.fract() == 0.0 => Ok(*n as u64),
        _ => Err(query_error(format!(
            "{}() expects a single non-negative integer",
            call.name
        ))),
    }
}

/// `{ upsert: true }` style options for update/replace
fn upsert_option(call: &MethodCall, index: usize) -> Result<Option<bool>, ConnectionError> {
    let options = optional_doc(call, index, "options")?;
    let mut upsert = None;
    for (key, value) in &options {
        match (key.as_str(), value) {
            ("upsert", Bson::Boolean(b)) => upsert = Some(*b),
            ("upsert", _) => return Err(query_error("upsert must be true or false")),
            (other, _) => {
                return Err(query_error(format!(
                    "Unsupported option '{other}' for {}() (supported: upsert)",
                    call.name
                )))
            }
        }
    }
    Ok(upsert)
}

/// Methods that only make sense on cursors and are no-ops for result display
fn is_display_noop(name: &str) -> bool {
    matches!(name, "toArray" | "pretty")
}

fn reject_chain(call: &MethodCall, chain: &[MethodCall]) -> Result<(), ConnectionError> {
    match chain.iter().find(|c| !is_display_noop(&c.name)) {
        Some(c) => Err(query_error(format!(
            "'.{}()' cannot follow {}()",
            c.name, call.name
        ))),
        None => Ok(()),
    }
}

// ============================================================================
// Results
// ============================================================================

/// Turn documents into a grid: one column per top-level field, in first-seen
/// order with `_id` first. Missing fields are NULL.
pub fn documents_to_result_set(docs: &[Document], truncated: bool) -> ResultSet {
    let mut names: Vec<String> = Vec::new();
    let mut types: Vec<Option<&'static str>> = Vec::new();
    if docs.iter().any(|d| d.contains_key("_id")) {
        names.push("_id".to_string());
        types.push(None);
    }
    for doc in docs {
        for (key, value) in doc {
            let idx = match names.iter().position(|n| n == key) {
                Some(idx) => idx,
                None => {
                    names.push(key.clone());
                    types.push(None);
                    names.len() - 1
                }
            };
            if types[idx].is_none() && !matches!(value, Bson::Null) {
                types[idx] = Some(type_name(value));
            }
        }
    }

    let rows = docs
        .iter()
        .map(|doc| {
            names
                .iter()
                .map(|name| {
                    doc.get(name)
                        .map(super::convert::to_cell)
                        .unwrap_or(crate::db::query::CellValue::Null)
                })
                .collect()
        })
        .collect();

    let columns = names
        .into_iter()
        .zip(types)
        .map(|(name, ty)| ColumnInfo {
            name,
            data_type: ty.unwrap_or("null").to_string(),
            nullable: true,
        })
        .collect();

    ResultSet {
        columns,
        rows,
        truncated,
    }
}

fn single_value_set(column: &str, values: Vec<Bson>, truncated: bool) -> ResultSet {
    let docs: Vec<Document> = values.into_iter().map(|v| doc! { column: v }).collect();
    documents_to_result_set(&docs, truncated)
}

fn result_sets(set: ResultSet, row_limit: usize) -> ExecutionOutput {
    ExecutionOutput::ResultSets {
        sets: vec![set],
        row_limit: Some(row_limit),
    }
}

/// Drain a cursor up to `row_limit + 1` documents; the extra one marks truncation.
/// Dropping the cursor afterwards makes the driver kill it server-side.
async fn collect(
    mut cursor: Cursor<Document>,
    row_limit: usize,
) -> Result<(Vec<Document>, bool), ConnectionError> {
    let mut docs = Vec::new();
    while let Some(doc) = cursor.try_next().await? {
        docs.push(doc);
        if docs.len() > row_limit {
            docs.truncate(row_limit);
            return Ok((docs, true));
        }
    }
    Ok((docs, false))
}

// ============================================================================
// Execution
// ============================================================================

/// Execute one parsed statement.
/// `database` is the tab's database (already defaulted by the caller).
pub async fn execute(
    client: &Client,
    database: &str,
    statement: Statement,
    row_limit: usize,
) -> Result<ExecutionOutput, ConnectionError> {
    match statement {
        Statement::Use(_) => Ok(ExecutionOutput::ResultSets {
            // The editor switches the tab's database; nothing to run server-side
            sets: Vec::new(),
            row_limit: None,
        }),
        Statement::Show(ShowTarget::Databases) => {
            let names = client
                .list_database_names()
                .authorized_databases(true)
                .await?;
            let values = names.into_iter().map(Bson::String).collect();
            Ok(result_sets(
                single_value_set("name", values, false),
                row_limit,
            ))
        }
        Statement::Show(ShowTarget::Collections) => {
            let mut names = client.database(database).list_collection_names().await?;
            names.sort();
            let values = names.into_iter().map(Bson::String).collect();
            Ok(result_sets(
                single_value_set("name", values, false),
                row_limit,
            ))
        }
        Statement::DatabaseCall {
            database: sibling,
            call,
        } => {
            let db = client.database(sibling.as_deref().unwrap_or(database));
            execute_database_call(client, &db, call, row_limit).await
        }
        Statement::CollectionCall {
            database: sibling,
            collection,
            call,
            chain,
        } => {
            let db = client.database(sibling.as_deref().unwrap_or(database));
            let coll = db.collection::<Document>(&collection);
            execute_collection_call(&coll, call, chain, row_limit).await
        }
    }
}

async fn execute_database_call(
    client: &Client,
    db: &Database,
    call: MethodCall,
    row_limit: usize,
) -> Result<ExecutionOutput, ConnectionError> {
    match call.name.as_str() {
        "runCommand" | "adminCommand" => {
            check_arity(&call, 1)?;
            let command = required_doc(&call, 0, "a command document")?;
            let target = if call.name == "adminCommand" {
                client.database("admin")
            } else {
                db.clone()
            };
            let reply = target.run_command(command).await?;
            Ok(result_sets(documents_to_result_set(&[reply], false), row_limit))
        }
        "getCollectionNames" => {
            check_arity(&call, 0)?;
            let mut names = db.list_collection_names().await?;
            names.sort();
            let values = names.into_iter().map(Bson::String).collect();
            Ok(result_sets(single_value_set("name", values, false), row_limit))
        }
        "stats" => {
            check_arity(&call, 0)?;
            let reply = db.run_command(doc! { "dbStats": 1 }).await?;
            Ok(result_sets(documents_to_result_set(&[reply], false), row_limit))
        }
        other => Err(query_error(format!(
            "Unsupported database method 'db.{other}()' (supported: runCommand, adminCommand, getCollectionNames, stats). \
             If '{other}' is a collection, call a method on it, e.g. db.{other}.find()"
        ))),
    }
}

async fn execute_collection_call(
    coll: &Collection<Document>,
    call: MethodCall,
    chain: Vec<MethodCall>,
    row_limit: usize,
) -> Result<ExecutionOutput, ConnectionError> {
    match call.name.as_str() {
        "find" => execute_find(coll, call, chain, row_limit).await,
        "findOne" => {
            check_arity(&call, 2)?;
            reject_chain(&call, &chain)?;
            let filter = optional_doc(&call, 0, "filter")?;
            let mut options = FindOneOptions::default();
            if call.args.len() > 1 {
                options.projection = Some(optional_doc(&call, 1, "projection")?);
            }
            let doc = coll.find_one(filter).with_options(options).await?;
            let docs: Vec<Document> = doc.into_iter().collect();
            Ok(result_sets(
                documents_to_result_set(&docs, false),
                row_limit,
            ))
        }
        "aggregate" => {
            check_arity(&call, 1)?;
            reject_chain(&call, &chain)?;
            let pipeline = match call.args.first() {
                None => Vec::new(),
                Some(value) => doc_list(value, "pipeline")?,
            };
            let cursor = coll.aggregate(pipeline).await?;
            let (docs, truncated) = collect(cursor, row_limit).await?;
            Ok(result_sets(
                documents_to_result_set(&docs, truncated),
                row_limit,
            ))
        }
        "countDocuments" | "count" => {
            check_arity(&call, 1)?;
            reject_chain(&call, &chain)?;
            let filter = optional_doc(&call, 0, "filter")?;
            let count = coll.count_documents(filter).await?;
            let value = Bson::Int64(i64::try_from(count).unwrap_or(i64::MAX));
            Ok(result_sets(
                single_value_set("count", vec![value], false),
                row_limit,
            ))
        }
        "estimatedDocumentCount" => {
            check_arity(&call, 0)?;
            reject_chain(&call, &chain)?;
            let count = coll.estimated_document_count().await?;
            let value = Bson::Int64(i64::try_from(count).unwrap_or(i64::MAX));
            Ok(result_sets(
                single_value_set("count", vec![value], false),
                row_limit,
            ))
        }
        "distinct" => {
            check_arity(&call, 2)?;
            reject_chain(&call, &chain)?;
            let field = match call.args.first() {
                Some(Value::Str(field)) => field.clone(),
                _ => return Err(query_error("distinct() expects a field name string first")),
            };
            let filter = optional_doc(&call, 1, "filter")?;
            let mut values = coll.distinct(&field, filter).await?;
            let truncated = values.len() > row_limit;
            values.truncate(row_limit);
            Ok(result_sets(
                single_value_set(&field, values, truncated),
                row_limit,
            ))
        }
        "insertOne" => {
            check_arity(&call, 1)?;
            reject_chain(&call, &chain)?;
            let doc = required_doc(&call, 0, "a document")?;
            coll.insert_one(doc).await?;
            Ok(ExecutionOutput::RowsAffected(1))
        }
        "insertMany" => {
            check_arity(&call, 1)?;
            reject_chain(&call, &chain)?;
            let docs = match call.args.first() {
                Some(value) => doc_list(value, "documents")?,
                None => return Err(query_error("insertMany() requires an array of documents")),
            };
            if docs.is_empty() {
                return Err(query_error("insertMany() requires at least one document"));
            }
            let result = coll.insert_many(docs).await?;
            Ok(ExecutionOutput::RowsAffected(result.inserted_ids.len()))
        }
        "updateOne" | "updateMany" => {
            check_arity(&call, 3)?;
            reject_chain(&call, &chain)?;
            let filter = required_doc(&call, 0, "a filter")?;
            let update: mongodb::options::UpdateModifications = match call.args.get(1) {
                Some(value @ Value::Array(_)) => doc_list(value, "update pipeline")?.into(),
                Some(value) => to_document(value, "update").map_err(query_error)?.into(),
                None => return Err(query_error(format!("{}() requires an update", call.name))),
            };
            let upsert = upsert_option(&call, 2)?;
            let result = if call.name == "updateOne" {
                coll.update_one(filter, update)
                    .upsert(upsert.unwrap_or(false))
                    .await?
            } else {
                coll.update_many(filter, update)
                    .upsert(upsert.unwrap_or(false))
                    .await?
            };
            let upserted = usize::from(result.upserted_id.is_some());
            Ok(ExecutionOutput::RowsAffected(
                result.modified_count as usize + upserted,
            ))
        }
        "replaceOne" => {
            check_arity(&call, 3)?;
            reject_chain(&call, &chain)?;
            let filter = required_doc(&call, 0, "a filter")?;
            let replacement = required_doc(&call, 1, "a replacement document")?;
            let upsert = upsert_option(&call, 2)?;
            let result = coll
                .replace_one(filter, replacement)
                .upsert(upsert.unwrap_or(false))
                .await?;
            let upserted = usize::from(result.upserted_id.is_some());
            Ok(ExecutionOutput::RowsAffected(
                result.modified_count as usize + upserted,
            ))
        }
        "deleteOne" | "deleteMany" => {
            check_arity(&call, 1)?;
            reject_chain(&call, &chain)?;
            // An explicit filter is required so `deleteMany()` can't wipe a collection by accident
            let filter = required_doc(&call, 0, "a filter ({} deletes everything)")?;
            let result = if call.name == "deleteOne" {
                coll.delete_one(filter).await?
            } else {
                coll.delete_many(filter).await?
            };
            Ok(ExecutionOutput::RowsAffected(result.deleted_count as usize))
        }
        "getIndexes" => {
            check_arity(&call, 0)?;
            reject_chain(&call, &chain)?;
            let indexes: Vec<Document> = coll
                .list_indexes()
                .await?
                .try_collect::<Vec<_>>()
                .await?
                .into_iter()
                .filter_map(|index| mongodb::bson::to_document(&index).ok())
                .collect();
            Ok(result_sets(
                documents_to_result_set(&indexes, false),
                row_limit,
            ))
        }
        other => Err(query_error(format!(
            "Unsupported collection method '{other}()' (supported: {})",
            COLLECTION_METHODS.join(", ")
        ))),
    }
}

async fn execute_find(
    coll: &Collection<Document>,
    call: MethodCall,
    chain: Vec<MethodCall>,
    row_limit: usize,
) -> Result<ExecutionOutput, ConnectionError> {
    check_arity(&call, 2)?;
    let filter = optional_doc(&call, 0, "filter")?;
    let mut options = FindOptions::default();
    if call.args.len() > 1 {
        options.projection = Some(optional_doc(&call, 1, "projection")?);
    }

    let mut user_limit: Option<u64> = None;
    let mut count_only = false;
    for link in &chain {
        match link.name.as_str() {
            "sort" => {
                check_arity(link, 1)?;
                options.sort = Some(required_doc(link, 0, "a sort document")?);
            }
            "limit" => user_limit = Some(non_negative(link)?),
            "skip" => options.skip = Some(non_negative(link)?),
            "projection" | "project" => {
                check_arity(link, 1)?;
                options.projection = Some(required_doc(link, 0, "a projection document")?);
            }
            "maxTimeMS" => options.max_time = Some(Duration::from_millis(non_negative(link)?)),
            "count" => count_only = true,
            "batchSize" => {
                let size = non_negative(link)?;
                options.batch_size = u32::try_from(size).ok();
            }
            name if is_display_noop(name) => {}
            other => {
                return Err(query_error(format!(
                    "Unsupported cursor method '.{other}()' (supported: sort, limit, skip, projection, maxTimeMS, count, toArray)"
                )))
            }
        }
    }

    if count_only {
        let count = coll.count_documents(filter).await?;
        let value = Bson::Int64(i64::try_from(count).unwrap_or(i64::MAX));
        return Ok(result_sets(
            single_value_set("count", vec![value], false),
            row_limit,
        ));
    }

    // Fetch one extra row (when the user's limit allows) to detect truncation
    let fetch = (row_limit as u64).saturating_add(1);
    let limit = match user_limit {
        Some(0) | None => fetch,
        Some(n) => n.min(fetch),
    };
    options.limit = Some(i64::try_from(limit).unwrap_or(i64::MAX));

    let cursor = coll.find(filter).with_options(options).await?;
    let (docs, truncated) = collect(cursor, row_limit).await?;
    Ok(result_sets(
        documents_to_result_set(&docs, truncated),
        row_limit,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::query::CellValue;

    #[test]
    fn documents_become_columns_in_first_seen_order_with_id_first() {
        let docs = vec![
            doc! { "name": "a", "_id": 1, "age": 30 },
            doc! { "_id": 2, "email": "b@example.test", "age": Bson::Null },
        ];
        let set = documents_to_result_set(&docs, false);
        let names: Vec<&str> = set.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["_id", "name", "age", "email"]);
        assert_eq!(set.columns[2].data_type, "int");
        assert_eq!(set.rows.len(), 2);
        assert!(matches!(set.rows[1][1], CellValue::Null)); // missing name
        assert!(matches!(set.rows[1][3], CellValue::String(ref s) if s == "b@example.test"));
    }

    #[test]
    fn upsert_option_validates_keys() {
        let call = |args| MethodCall {
            name: "updateOne".into(),
            args,
        };
        let ok = call(vec![
            Value::Object(vec![]),
            Value::Object(vec![]),
            Value::Object(vec![("upsert".into(), Value::Bool(true))]),
        ]);
        assert_eq!(upsert_option(&ok, 2).unwrap(), Some(true));
        let bad = call(vec![
            Value::Object(vec![]),
            Value::Object(vec![]),
            Value::Object(vec![("writeConcern".into(), Value::Object(vec![]))]),
        ]);
        assert!(upsert_option(&bad, 2).is_err());
    }

    #[test]
    fn chain_rules() {
        let call = MethodCall {
            name: "aggregate".into(),
            args: vec![],
        };
        let pretty = MethodCall {
            name: "pretty".into(),
            args: vec![],
        };
        let sort = MethodCall {
            name: "sort".into(),
            args: vec![],
        };
        assert!(reject_chain(&call, &[pretty]).is_ok());
        assert!(reject_chain(&call, &[sort]).is_err());
    }
}
