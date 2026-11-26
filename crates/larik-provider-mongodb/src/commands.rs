use mongodb::bson::Document;

#[derive(Debug, Clone, PartialEq)]
pub enum MongoCommand {
    Select {
        collection: String,
        pipeline: Vec<Document>,
        explicit_columns: Vec<String>,
        limit: Option<i64>,
    },
    Insert {
        collection: String,
        documents: Vec<Document>,
    },
    Update {
        collection: String,
        filter: Document,
        set_fields: Document,
        unset_fields: Document,
    },
    Delete {
        collection: String,
        filter: Document,
        has_where: bool,
    },
    Use {
        database: String,
    },
}
