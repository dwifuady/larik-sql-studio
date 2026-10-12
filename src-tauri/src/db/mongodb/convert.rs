// Conversions: shell `Value` -> BSON, and BSON -> result-grid cells.

use base64::Engine as _;
use mongodb::bson::{
    oid::ObjectId, spec::BinarySubtype, Binary, Bson, DateTime, Decimal128, Document, Regex,
    Timestamp,
};

use super::shell::Value;
use crate::db::query::CellValue;

/// Convert a parsed shell value to BSON, evaluating helper calls such as
/// `ObjectId("...")` and `ISODate("...")`.
pub fn to_bson(value: &Value) -> Result<Bson, String> {
    Ok(match value {
        Value::Null => Bson::Null,
        Value::Undefined => Bson::Undefined,
        Value::Bool(b) => Bson::Boolean(*b),
        // Like mongosh: integers that fit are stored as int32
        Value::Int(v) => match i32::try_from(*v) {
            Ok(small) => Bson::Int32(small),
            Err(_) => Bson::Int64(*v),
        },
        Value::Double(v) => Bson::Double(*v),
        Value::Str(s) => Bson::String(s.clone()),
        Value::Regex { pattern, flags } => Bson::RegularExpression(Regex {
            pattern: pattern.clone(),
            options: sorted_flags(flags),
        }),
        Value::Array(items) => Bson::Array(items.iter().map(to_bson).collect::<Result<_, _>>()?),
        Value::Object(fields) => Bson::Document(to_document_fields(fields)?),
        Value::Call { name, args } => call_to_bson(name, args)?,
    })
}

/// Convert a value that must be an object (filter, update, pipeline stage...)
pub fn to_document(value: &Value, what: &str) -> Result<Document, String> {
    match value {
        Value::Object(fields) => to_document_fields(fields),
        _ => Err(format!("{what} must be an object, e.g. {{ field: value }}")),
    }
}

fn to_document_fields(fields: &[(String, Value)]) -> Result<Document, String> {
    let mut doc = Document::new();
    for (key, value) in fields {
        doc.insert(key.clone(), to_bson(value)?);
    }
    Ok(doc)
}

/// BSON requires regex options in alphabetical order
fn sorted_flags(flags: &str) -> String {
    let mut chars: Vec<char> = flags.chars().collect();
    chars.sort_unstable();
    chars.dedup();
    chars.into_iter().collect()
}

fn string_arg<'a>(name: &str, args: &'a [Value]) -> Result<&'a str, String> {
    match args {
        [Value::Str(s)] => Ok(s),
        _ => Err(format!("{name}() expects a single string argument")),
    }
}

fn parse_date(s: &str) -> Result<DateTime, String> {
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
    let s = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(DateTime::from_millis(dt.timestamp_millis()));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(DateTime::from_millis(
                Utc.from_utc_datetime(&naive).timestamp_millis(),
            ));
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let naive = date.and_hms_opt(0, 0, 0).ok_or("Invalid date")?;
        return Ok(DateTime::from_millis(
            Utc.from_utc_datetime(&naive).timestamp_millis(),
        ));
    }
    Err(format!(
        "Invalid date '{s}' (use ISO 8601, e.g. 2024-01-31T12:00:00Z)"
    ))
}

fn integer_arg(name: &str, args: &[Value]) -> Result<i64, String> {
    match args {
        [Value::Int(v)] => Ok(*v),
        [Value::Double(v)] if v.fract() == 0.0 => Ok(*v as i64),
        [Value::Str(s)] => s
            .trim()
            .parse::<i64>()
            .map_err(|_| format!("{name}() expects an integer, got '{s}'")),
        _ => Err(format!("{name}() expects a single integer argument")),
    }
}

fn call_to_bson(name: &str, args: &[Value]) -> Result<Bson, String> {
    match name {
        "ObjectId" => match args {
            [] => Ok(Bson::ObjectId(ObjectId::new())),
            _ => {
                let hex = string_arg(name, args)?;
                ObjectId::parse_str(hex)
                    .map(Bson::ObjectId)
                    .map_err(|_| format!("Invalid ObjectId '{hex}' (expected 24 hex characters)"))
            }
        },
        "ISODate" | "Date" => match args {
            [] => Ok(Bson::DateTime(DateTime::now())),
            [Value::Int(ms)] => Ok(Bson::DateTime(DateTime::from_millis(*ms))),
            [Value::Double(ms)] => Ok(Bson::DateTime(DateTime::from_millis(*ms as i64))),
            _ => parse_date(string_arg(name, args)?).map(Bson::DateTime),
        },
        "NumberInt" | "Int32" => {
            let v = integer_arg(name, args)?;
            i32::try_from(v)
                .map(Bson::Int32)
                .map_err(|_| format!("{name}({v}) is out of int32 range"))
        }
        "NumberLong" | "Long" => integer_arg(name, args).map(Bson::Int64),
        "NumberDecimal" | "Decimal128" => {
            let text = match args {
                [Value::Str(s)] => s.clone(),
                [Value::Int(v)] => v.to_string(),
                [Value::Double(v)] => v.to_string(),
                _ => return Err(format!("{name}() expects a single number or string")),
            };
            text.parse::<Decimal128>()
                .map(Bson::Decimal128)
                .map_err(|_| format!("Invalid decimal '{text}'"))
        }
        "Double" => match args {
            [Value::Int(v)] => Ok(Bson::Double(*v as f64)),
            [Value::Double(v)] => Ok(Bson::Double(*v)),
            _ => Err("Double() expects a single number".to_string()),
        },
        "UUID" => {
            let text = string_arg(name, args)?;
            let hex: String = text.chars().filter(|c| *c != '-').collect();
            if hex.len() != 32 {
                return Err(format!("Invalid UUID '{text}'"));
            }
            let bytes = (0..16)
                .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|_| format!("Invalid UUID '{text}'"))?;
            Ok(Bson::Binary(Binary {
                subtype: BinarySubtype::Uuid,
                bytes,
            }))
        }
        "BinData" => match args {
            [Value::Int(subtype), Value::Str(b64)] => {
                let subtype = u8::try_from(*subtype)
                    .map_err(|_| "BinData subtype must be 0-255".to_string())?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|_| "BinData expects base64 data".to_string())?;
                Ok(Bson::Binary(Binary {
                    subtype: BinarySubtype::from(subtype),
                    bytes,
                }))
            }
            _ => Err("BinData() expects (subtype, \"base64\")".to_string()),
        },
        "Timestamp" => match args {
            [Value::Int(t), Value::Int(i)] => Ok(Bson::Timestamp(Timestamp {
                time: u32::try_from(*t).map_err(|_| "Timestamp time out of range")?,
                increment: u32::try_from(*i).map_err(|_| "Timestamp increment out of range")?,
            })),
            _ => Err("Timestamp() expects (seconds, increment)".to_string()),
        },
        "RegExp" => match args {
            [Value::Str(pattern)] => Ok(Bson::RegularExpression(Regex {
                pattern: pattern.clone(),
                options: String::new(),
            })),
            [Value::Str(pattern), Value::Str(flags)] => Ok(Bson::RegularExpression(Regex {
                pattern: pattern.clone(),
                options: sorted_flags(flags),
            })),
            _ => Err("RegExp() expects (pattern[, flags])".to_string()),
        },
        "MinKey" => Ok(Bson::MinKey),
        "MaxKey" => Ok(Bson::MaxKey),
        other => Err(format!("Unsupported function '{other}()'")),
    }
}

/// Short type name shown in result-grid headers and the schema explorer
pub fn type_name(value: &Bson) -> &'static str {
    match value {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Array(_) => "array",
        Bson::Document(_) => "object",
        Bson::Boolean(_) => "bool",
        Bson::Null => "null",
        Bson::Undefined => "undefined",
        Bson::RegularExpression(_) => "regex",
        Bson::JavaScriptCode(_) | Bson::JavaScriptCodeWithScope(_) => "javascript",
        Bson::Int32(_) => "int",
        Bson::Int64(_) => "long",
        Bson::Timestamp(_) => "timestamp",
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid => "uuid",
        Bson::Binary(_) => "binData",
        Bson::ObjectId(_) => "objectId",
        Bson::DateTime(_) => "date",
        Bson::Symbol(_) => "symbol",
        Bson::Decimal128(_) => "decimal",
        Bson::MaxKey => "maxKey",
        Bson::MinKey => "minKey",
        Bson::DbPointer(_) => "dbPointer",
    }
}

fn format_uuid(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if hex.len() != 32 {
        return hex;
    }
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Convert a BSON value to a result-grid cell.
/// Nested documents and arrays are rendered as relaxed Extended JSON text.
pub fn to_cell(value: &Bson) -> CellValue {
    match value {
        Bson::Null | Bson::Undefined => CellValue::Null,
        Bson::Boolean(b) => CellValue::Bool(*b),
        Bson::Int32(v) => CellValue::Int(i64::from(*v)),
        Bson::Int64(v) => CellValue::Int(*v),
        Bson::Double(v) => CellValue::Float(*v),
        Bson::String(s) | Bson::Symbol(s) => CellValue::String(s.clone()),
        Bson::ObjectId(oid) => CellValue::String(oid.to_hex()),
        Bson::DateTime(dt) => CellValue::DateTime(
            dt.try_to_rfc3339_string()
                .unwrap_or_else(|_| dt.timestamp_millis().to_string()),
        ),
        Bson::Decimal128(d) => CellValue::String(d.to_string()),
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid => {
            CellValue::String(format_uuid(&b.bytes))
        }
        Bson::Binary(b) => CellValue::Binary(b.bytes.clone()),
        Bson::RegularExpression(r) => CellValue::String(format!("/{}/{}", r.pattern, r.options)),
        Bson::Timestamp(t) => CellValue::String(format!("Timestamp({}, {})", t.time, t.increment)),
        other => CellValue::String(other.clone().into_relaxed_extjson().to_string()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::mongodb::shell::{parse_single, Statement};
    use mongodb::bson::doc;

    fn first_arg(src: &str) -> Document {
        let Statement::CollectionCall { call, .. } = parse_single(src).unwrap() else {
            panic!()
        };
        to_document(&call.args[0], "filter").unwrap()
    }

    #[test]
    fn converts_filter_with_operators_and_helpers() {
        let doc = first_arg(
            r#"db.c.find({ age: { $gt: 30 }, _id: ObjectId("65a1f0c2e4b0a1b2c3d4e5f6"),
                at: ISODate("2024-01-01"), n: /ab/mi, big: NumberLong(5), d: NumberDecimal("1.10") })"#,
        );
        assert_eq!(doc.get_document("age").unwrap(), &doc! { "$gt": 30 });
        assert_eq!(
            doc.get_object_id("_id").unwrap().to_hex(),
            "65a1f0c2e4b0a1b2c3d4e5f6"
        );
        assert_eq!(
            doc.get_datetime("at").unwrap().timestamp_millis(),
            1_704_067_200_000
        );
        assert_eq!(
            doc.get("n"),
            Some(&Bson::RegularExpression(Regex {
                pattern: "ab".into(),
                options: "im".into()
            }))
        );
        assert_eq!(doc.get("big"), Some(&Bson::Int64(5)));
        assert_eq!(doc.get("d").map(type_name), Some("decimal"));
    }

    #[test]
    fn integers_use_int32_when_they_fit() {
        assert_eq!(to_bson(&Value::Int(7)).unwrap(), Bson::Int32(7));
        assert_eq!(
            to_bson(&Value::Int(5_000_000_000)).unwrap(),
            Bson::Int64(5_000_000_000)
        );
    }

    #[test]
    fn rejects_bad_helpers() {
        assert!(to_bson(&Value::Call {
            name: "ObjectId".into(),
            args: vec![Value::Str("nope".into())]
        })
        .is_err());
        assert!(to_bson(&Value::Call {
            name: "Function".into(),
            args: vec![]
        })
        .is_err());
        assert!(to_document(&Value::Int(1), "filter").is_err());
    }

    #[test]
    fn cells_render_mongo_types() {
        let oid = ObjectId::parse_str("65a1f0c2e4b0a1b2c3d4e5f6").unwrap();
        assert!(
            matches!(to_cell(&Bson::ObjectId(oid)), CellValue::String(s) if s == "65a1f0c2e4b0a1b2c3d4e5f6")
        );
        assert!(matches!(to_cell(&Bson::Int32(3)), CellValue::Int(3)));
        assert!(matches!(to_cell(&Bson::Null), CellValue::Null));
        let nested = to_cell(&Bson::Document(doc! { "city": "Sydney" }));
        assert!(matches!(nested, CellValue::String(s) if s == r#"{"city":"Sydney"}"#));
        let uuid = to_bson(&Value::Call {
            name: "UUID".into(),
            args: vec![Value::Str("123e4567-e89b-12d3-a456-426614174000".into())],
        })
        .unwrap();
        assert!(
            matches!(to_cell(&uuid), CellValue::String(s) if s == "123e4567-e89b-12d3-a456-426614174000")
        );
    }
}
