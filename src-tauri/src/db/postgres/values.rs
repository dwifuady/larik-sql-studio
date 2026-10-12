// tokio-postgres -> driver-neutral value conversion
// Maps PostgreSQL column types onto `CellValue` / `ColumnInfo`, mirroring the
// conventions of `db/mssql/values.rs` (dates as `DateTime` strings, binary as
// `CellValue::Binary`, numeric as `Float`).

use crate::db::query::{CellValue, ColumnInfo};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use tokio_postgres::types::{Kind, Type};
use tokio_postgres::{Column, Row};

/// Decode a cell for the PostgreSQL type `ty` at column `idx` of `row`.
///
/// Every arm uses `try_get::<_, Option<T>>` so SQL `NULL` collapses to
/// `CellValue::Null`. Unknown/custom types fall back to a raw-bytes decoder:
/// text-decodable values (custom enums, text-compatible types) become strings,
/// everything else becomes a `Binary` cell instead of failing the whole row.
pub(super) fn cell_from_row(row: &Row, idx: usize, ty: &Type) -> CellValue {
    match *ty {
        Type::BOOL => get::<bool>(row, idx, CellValue::Bool),
        Type::INT2 => get::<i16>(row, idx, |v| CellValue::Int(i64::from(v))),
        Type::OID => get::<u32>(row, idx, |v| CellValue::Int(i64::from(v))),
        Type::INT4 => get::<i32>(row, idx, |v| CellValue::Int(i64::from(v))),
        Type::INT8 => get::<i64>(row, idx, CellValue::Int),
        Type::CHAR => get::<i8>(row, idx, |v| CellValue::Int(i64::from(v))), // internal 1-byte "char"
        Type::FLOAT4 => get::<f32>(row, idx, |v| CellValue::Float(f64::from(v))),
        Type::FLOAT8 => get::<f64>(row, idx, CellValue::Float),
        Type::NUMERIC => get::<Decimal>(row, idx, |d| {
            d.to_f64().map(CellValue::Float).unwrap_or(CellValue::Null)
        }),
        Type::MONEY => get::<i64>(row, idx, |v| CellValue::Float(v as f64 / 100.0)),

        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => get_str(row, idx),

        Type::UUID => get::<uuid::Uuid>(row, idx, |u| CellValue::String(u.to_string())),
        Type::JSON | Type::JSONB => get::<serde_json::Value>(row, idx, |v| {
            CellValue::String(v.to_string())
        }),

        Type::DATE => get::<NaiveDate>(row, idx, |d| CellValue::DateTime(d.to_string())),
        Type::TIME => get::<NaiveTime>(row, idx, |t| CellValue::DateTime(t.to_string())),
        Type::TIMESTAMP => {
            get::<NaiveDateTime>(row, idx, |dt| CellValue::DateTime(dt.to_string()))
        }
        Type::TIMESTAMPTZ => {
            get::<DateTime<Utc>>(row, idx, |dt| CellValue::DateTime(dt.to_rfc3339()))
        }

        Type::INET | Type::CIDR => get::<std::net::IpAddr>(row, idx, |ip| {
            CellValue::String(ip.to_string())
        }),

        Type::VOID => CellValue::String(String::new()),
        Type::BYTEA => get::<Vec<u8>>(row, idx, CellValue::Binary),

        // Arrays of common element types; anything else falls through to the
        // raw-bytes fallback below.
        _ if matches!(ty.kind(), Kind::Array(_)) => array_cell(row, idx, ty),

        _ => raw_cell(row, idx),
    }
}

fn get<T>(row: &Row, idx: usize, map: impl Fn(T) -> CellValue) -> CellValue
where
    T: for<'a> tokio_postgres::types::FromSql<'a>,
{
    match row.try_get::<_, Option<T>>(idx) {
        Ok(Some(v)) => map(v),
        _ => CellValue::Null,
    }
}

fn get_str(row: &Row, idx: usize) -> CellValue {
    match row.try_get::<_, Option<&str>>(idx) {
        Ok(Some(s)) => CellValue::String(s.to_string()),
        _ => CellValue::Null,
    }
}

/// Last-resort decode for custom/unknown types: accept every OID, try UTF-8
/// text, else surface the raw bytes as a binary cell.
fn raw_cell(row: &Row, idx: usize) -> CellValue {
    match row.try_get::<_, Option<AnyText>>(idx) {
        Ok(Some(AnyText(Some(s)))) => CellValue::String(s),
        _ => match row.try_get::<_, Option<Vec<u8>>>(idx) {
            Ok(Some(bytes)) => CellValue::Binary(bytes),
            _ => CellValue::Null,
        },
    }
}

struct AnyText(Option<String>);

impl<'a> tokio_postgres::types::FromSql<'a> for AnyText {
    fn from_sql(
        _: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(AnyText(std::str::from_utf8(raw).ok().map(str::to_string)))
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    fn from_sql_null(_: &Type) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(AnyText(None))
    }
}

/// Decode arrays of common element types into a PostgreSQL-style literal
/// (`{1,2,3}`, `{"a","b"}`). Unknown element types fall back to `raw_cell`.
fn array_cell(row: &Row, idx: usize, ty: &Type) -> CellValue {
    let Some(elem) = (match ty.kind() {
        Kind::Array(elem) => Some(elem.clone()),
        _ => None,
    }) else {
        return raw_cell(row, idx);
    };
    let rendered: Vec<String> = match elem {
        Type::BOOL => match row.try_get::<_, Option<Vec<bool>>>(idx) {
            Ok(Some(v)) => v.iter().map(ToString::to_string).collect(),
            _ => return CellValue::Null,
        },
        Type::INT2 => match row.try_get::<_, Option<Vec<i16>>>(idx) {
            Ok(Some(v)) => v.iter().map(ToString::to_string).collect(),
            _ => return CellValue::Null,
        },
        Type::INT4 => match row.try_get::<_, Option<Vec<i32>>>(idx) {
            Ok(Some(v)) => v.iter().map(ToString::to_string).collect(),
            _ => return CellValue::Null,
        },
        Type::INT8 => match row.try_get::<_, Option<Vec<i64>>>(idx) {
            Ok(Some(v)) => v.iter().map(ToString::to_string).collect(),
            _ => return CellValue::Null,
        },
        Type::FLOAT4 => match row.try_get::<_, Option<Vec<f32>>>(idx) {
            Ok(Some(v)) => v.iter().map(ToString::to_string).collect(),
            _ => return CellValue::Null,
        },
        Type::FLOAT8 => match row.try_get::<_, Option<Vec<f64>>>(idx) {
            Ok(Some(v)) => v.iter().map(ToString::to_string).collect(),
            _ => return CellValue::Null,
        },
        Type::NUMERIC => match row.try_get::<_, Option<Vec<Decimal>>>(idx) {
            Ok(Some(v)) => v.iter().map(ToString::to_string).collect(),
            _ => return CellValue::Null,
        },
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
            match row.try_get::<_, Option<Vec<String>>>(idx) {
                Ok(Some(v)) => v,
                _ => return CellValue::Null,
            }
        }
        Type::UUID => match row.try_get::<_, Option<Vec<uuid::Uuid>>>(idx) {
            Ok(Some(v)) => v.iter().map(|u| u.to_string()).collect(),
            _ => return CellValue::Null,
        },
        Type::JSON | Type::JSONB => {
            match row.try_get::<_, Option<Vec<serde_json::Value>>>(idx) {
                Ok(Some(v)) => v.iter().map(|j| j.to_string()).collect(),
                _ => return CellValue::Null,
            }
        }
        _ => return raw_cell(row, idx),
    };
    CellValue::String(format!("{{{}}}", render_array_elems(&rendered)))
}

/// Render elements of an array literal: `NULL` and numerics unquoted, strings
/// double-quoted with `\` and `"` escaped when they need it.
fn render_array_elems(elems: &[String]) -> String {
    elems
        .iter()
        .map(|e| {
            if e == "NULL" || is_numeric_literal(e) || is_bare_word(e) {
                e.clone()
            } else {
                format!("\"{}\"", e.replace('\\', "\\\\").replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn is_numeric_literal(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
}

fn is_bare_word(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Column metadata for a result column.
pub(super) fn column_info(col: &Column) -> ColumnInfo {
    ColumnInfo {
        name: col.name().to_string(),
        data_type: format_type(col.type_()),
        nullable: true, // row-level nullability is not advertised per result
    }
}

/// User-friendly type name: pretty names for the standard types, the pg name
/// for anything custom (`"my_enum"`), `elem[]` for arrays.
pub(super) fn format_type(ty: &Type) -> String {
    match *ty {
        Type::BOOL => "boolean".into(),
        Type::INT2 => "smallint".into(),
        Type::INT4 => "integer".into(),
        Type::INT8 => "bigint".into(),
        Type::OID => "oid".into(),
        Type::FLOAT4 => "real".into(),
        Type::FLOAT8 => "double precision".into(),
        Type::NUMERIC => "numeric".into(),
        Type::MONEY => "money".into(),
        Type::TEXT => "text".into(),
        Type::VARCHAR => "character varying".into(),
        Type::BPCHAR => "character".into(),
        Type::NAME => "name".into(),
        Type::UUID => "uuid".into(),
        Type::JSON => "json".into(),
        Type::JSONB => "jsonb".into(),
        Type::BYTEA => "bytea".into(),
        Type::DATE => "date".into(),
        Type::TIME => "time without time zone".into(),
        Type::TIMESTAMP => "timestamp without time zone".into(),
        Type::TIMESTAMPTZ => "timestamp with time zone".into(),
        Type::INET => "inet".into(),
        Type::CIDR => "cidr".into(),
        Type::CHAR => "char".into(),
        Type::VOID => "void".into(),
        Type::XML => "xml".into(),
        _ => match ty.kind() {
            Kind::Array(elem) => format!("{}[]", format_type(elem)),
            _ => ty.name().to_string(),
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn formats_standard_types() {
        assert_eq!(format_type(&Type::INT4), "integer");
        assert_eq!(format_type(&Type::TIMESTAMPTZ), "timestamp with time zone");
        assert_eq!(format_type(&Type::VARCHAR), "character varying");
    }

    #[test]
    fn formats_arrays() {
        assert_eq!(format_type(&Type::TEXT_ARRAY), "text[]");
        assert_eq!(format_type(&Type::INT4_ARRAY), "integer[]");
    }

    #[test]
    fn renders_array_literals() {
        assert_eq!(render_array_elems(&["1".into(), "2".into()]), "1,2");
        assert_eq!(
            render_array_elems(&["a b".into(), "NULL".into()]),
            "\"a b\",NULL"
        );
        assert_eq!(render_array_elems(&["x".into()]), "x");
    }
}
