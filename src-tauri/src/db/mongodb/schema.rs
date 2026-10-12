// Schema inference for MongoDB
// Collections map to tables; fields are inferred by sampling documents.

use futures::{StreamExt, TryStreamExt};
use mongodb::bson::{doc, Bson, Document};
use mongodb::{Client, Database};

use super::convert::type_name;
use crate::db::error::ConnectionError;
use crate::db::schema::{ColumnInfo, SchemaInfo, TableInfo};

/// Documents sampled per collection
const SAMPLE_SIZE: i64 = 100;
/// Nested document depth flattened into dotted field paths (`a.b.c`)
const MAX_DEPTH: usize = 3;
/// Collections sampled concurrently
const CONCURRENCY: usize = 8;

pub const TABLE_TYPE_COLLECTION: &str = "COLLECTION";
pub const TABLE_TYPE_VIEW: &str = "VIEW";

#[derive(Default)]
struct FieldStats {
    present: usize,
    nulls: usize,
    /// (type name, count) in first-seen order
    types: Vec<(&'static str, usize)>,
}

/// Accumulates field statistics over sampled documents
#[derive(Default)]
struct FieldCollector {
    order: Vec<String>,
    stats: std::collections::HashMap<String, FieldStats>,
    documents: usize,
}

impl FieldCollector {
    fn add_document(&mut self, doc: &Document) {
        self.documents += 1;
        let mut seen = std::collections::HashSet::new();
        self.walk(doc, "", 1, &mut seen);
    }

    fn record(&mut self, path: &str, value: &Bson, seen: &mut std::collections::HashSet<String>) {
        if !self.stats.contains_key(path) {
            self.order.push(path.to_string());
        }
        let stats = self.stats.entry(path.to_string()).or_default();
        // Count each path once per document (arrays of documents repeat paths)
        if seen.insert(path.to_string()) {
            stats.present += 1;
        }
        if matches!(value, Bson::Null) {
            stats.nulls += 1;
            return;
        }
        let name = type_name(value);
        match stats.types.iter_mut().find(|(t, _)| *t == name) {
            Some((_, count)) => *count += 1,
            None => stats.types.push((name, 1)),
        }
    }

    fn walk(
        &mut self,
        doc: &Document,
        prefix: &str,
        depth: usize,
        seen: &mut std::collections::HashSet<String>,
    ) {
        for (key, value) in doc {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            self.record(&path, value, seen);
            if depth >= MAX_DEPTH {
                continue;
            }
            match value {
                Bson::Document(inner) => self.walk(inner, &path, depth + 1, seen),
                // Arrays of documents: expose element fields as `array.field`
                Bson::Array(items) => {
                    for item in items.iter().take(20) {
                        if let Bson::Document(inner) = item {
                            self.walk(inner, &path, depth + 1, seen);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn into_columns(self) -> Vec<ColumnInfo> {
        let documents = self.documents;
        let mut stats = self.stats;
        let mut columns: Vec<ColumnInfo> = Vec::with_capacity(self.order.len());
        // `_id` first, then first-seen order
        let mut order = self.order;
        if let Some(pos) = order.iter().position(|p| p == "_id") {
            let id = order.remove(pos);
            order.insert(0, id);
        }
        for (index, path) in order.into_iter().enumerate() {
            let Some(field) = stats.remove(&path) else {
                continue;
            };
            let mut types = field.types;
            types.sort_by(|a, b| b.1.cmp(&a.1));
            let data_type = if types.is_empty() {
                "null".to_string()
            } else {
                types
                    .iter()
                    .map(|(t, _)| *t)
                    .collect::<Vec<_>>()
                    .join(" | ")
            };
            columns.push(ColumnInfo {
                is_primary_key: path == "_id",
                is_nullable: field.present < documents || field.nulls > 0,
                name: path,
                data_type,
                max_length: None,
                precision: None,
                scale: None,
                is_identity: false,
                is_computed: false,
                column_default: None,
                ordinal_position: i32::try_from(index + 1).unwrap_or(i32::MAX),
            });
        }
        columns
    }
}

/// Infer fields from a random sample of a collection
pub async fn sample_columns(
    db: &Database,
    collection: &str,
) -> Result<Vec<ColumnInfo>, ConnectionError> {
    let mut cursor = db
        .collection::<Document>(collection)
        .aggregate([doc! { "$sample": { "size": SAMPLE_SIZE } }])
        .await?;
    let mut collector = FieldCollector::default();
    while let Some(doc) = cursor.try_next().await? {
        collector.add_document(&doc);
    }
    Ok(collector.into_columns())
}

/// Collections and views of `database` with inferred fields
pub async fn fetch_schema(client: &Client, database: &str) -> Result<SchemaInfo, ConnectionError> {
    let db = client.database(database);
    let specs: Vec<_> = db.list_collections().await?.try_collect().await?;

    let mut entries: Vec<(String, String)> = specs
        .into_iter()
        .filter(|spec| !spec.name.starts_with("system."))
        .map(|spec| {
            let table_type = match spec.collection_type {
                mongodb::results::CollectionType::View => TABLE_TYPE_VIEW,
                _ => TABLE_TYPE_COLLECTION,
            };
            (spec.name, table_type.to_string())
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    // Sample concurrently; a collection we can't read still appears, without fields
    let tables: Vec<TableInfo> = futures::stream::iter(entries)
        .map(|(name, table_type)| {
            let db = db.clone();
            let schema_name = database.to_string();
            async move {
                let columns = sample_columns(&db, &name).await.unwrap_or_default();
                TableInfo {
                    schema_name,
                    table_name: name,
                    table_type,
                    columns,
                }
            }
        })
        .buffered(CONCURRENCY)
        .collect()
        .await;

    Ok(SchemaInfo {
        database_name: database.to_string(),
        // MongoDB has no schemas; the database stands in so explorer grouping works
        schemas: vec![database.to_string()],
        tables,
        relationships: Vec::new(),
        routines: Vec::new(),
        fetched_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn infers_paths_types_and_nullability() {
        let mut collector = FieldCollector::default();
        collector.add_document(&doc! {
            "name": "a", "_id": 1, "address": { "city": "Sydney", "geo": { "lat": 1.5, "deep": { "x": 1 } } },
            "items": [ { "sku": "x" }, { "sku": "y", "qty": 2 } ],
        });
        collector.add_document(&doc! { "_id": 2, "name": Bson::Null, "age": 40_i64 });
        let columns = collector.into_columns();
        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "_id",
                "name",
                "address",
                "address.city",
                "address.geo",
                "address.geo.lat",
                "address.geo.deep",
                "items",
                "items.sku",
                "items.qty",
                "age"
            ]
        );
        let get = |n: &str| columns.iter().find(|c| c.name == n).unwrap();
        assert!(get("_id").is_primary_key);
        assert!(!get("_id").is_nullable);
        assert!(get("name").is_nullable); // null in doc 2
        assert!(get("age").is_nullable); // missing in doc 1
        assert_eq!(get("age").data_type, "long");
        assert_eq!(get("address").data_type, "object");
        assert_eq!(get("items").data_type, "array");
        assert_eq!(get("address.geo.deep").data_type, "object");
        assert_eq!(get("_id").ordinal_position, 1);
    }

    #[test]
    fn mixed_types_are_listed_most_common_first() {
        let mut collector = FieldCollector::default();
        collector.add_document(&doc! { "v": "a" });
        collector.add_document(&doc! { "v": 1 });
        collector.add_document(&doc! { "v": 2 });
        let columns = collector.into_columns();
        assert_eq!(columns[0].data_type, "int | string");
    }
}
