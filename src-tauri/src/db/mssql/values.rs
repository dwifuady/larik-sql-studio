// tiberius -> driver-neutral value conversion
// Maps SQL Server column types onto `CellValue` / `ColumnInfo`.

use crate::db::query::{CellValue, ColumnInfo};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use tiberius::numeric::Numeric;
use tiberius::{Column, ColumnType, Row};

/// Convert a tiberius column value to CellValue
pub(super) fn cell_from_row(row: &Row, idx: usize, col_type: &ColumnType) -> CellValue {
    // Check for NULL first
    if row.try_get::<&str, _>(idx).ok().flatten().is_none()
        && row.try_get::<i32, _>(idx).ok().flatten().is_none()
        && row.try_get::<bool, _>(idx).ok().flatten().is_none()
    {
        // Try to detect if it's actually NULL
        match col_type {
            ColumnType::Null => return CellValue::Null,
            _ => {}
        }
    }

    match col_type {
        ColumnType::Null => CellValue::Null,

        // Integer types
        ColumnType::Int1 => row
            .try_get::<u8, _>(idx)
            .ok()
            .flatten()
            .map(|v| CellValue::Int(v as i64))
            .unwrap_or(CellValue::Null),
        ColumnType::Int2 => row
            .try_get::<i16, _>(idx)
            .ok()
            .flatten()
            .map(|v| CellValue::Int(v as i64))
            .unwrap_or(CellValue::Null),
        ColumnType::Int4 => row
            .try_get::<i32, _>(idx)
            .ok()
            .flatten()
            .map(|v| CellValue::Int(v as i64))
            .unwrap_or(CellValue::Null),
        ColumnType::Int8 => row
            .try_get::<i64, _>(idx)
            .ok()
            .flatten()
            .map(CellValue::Int)
            .unwrap_or(CellValue::Null),
        ColumnType::Intn => row
            .try_get::<i64, _>(idx)
            .ok()
            .flatten()
            .map(CellValue::Int)
            .or_else(|| {
                row.try_get::<i32, _>(idx)
                    .ok()
                    .flatten()
                    .map(|v| CellValue::Int(v as i64))
            })
            .unwrap_or(CellValue::Null),

        // Float types
        ColumnType::Float4 => row
            .try_get::<f32, _>(idx)
            .ok()
            .flatten()
            .map(|v| CellValue::Float(v as f64))
            .unwrap_or(CellValue::Null),
        ColumnType::Float8 => row
            .try_get::<f64, _>(idx)
            .ok()
            .flatten()
            .map(CellValue::Float)
            .unwrap_or(CellValue::Null),
        ColumnType::Floatn => row
            .try_get::<f64, _>(idx)
            .ok()
            .flatten()
            .map(CellValue::Float)
            .or_else(|| {
                row.try_get::<f32, _>(idx)
                    .ok()
                    .flatten()
                    .map(|v| CellValue::Float(v as f64))
            })
            .unwrap_or(CellValue::Null),

        // Decimal/Numeric types - use Numeric type from Tiberius
        ColumnType::Decimaln | ColumnType::Numericn => {
            row.try_get::<Numeric, _>(idx)
                .ok()
                .flatten()
                .map(|n| {
                    // Convert to f64 using From trait
                    CellValue::Float(f64::from(n))
                })
                .unwrap_or(CellValue::Null)
        }

        // Money types - Tiberius returns these as f64
        ColumnType::Money | ColumnType::Money4 => row
            .try_get::<f64, _>(idx)
            .ok()
            .flatten()
            .map(CellValue::Float)
            .unwrap_or(CellValue::Null),

        // Boolean
        ColumnType::Bit | ColumnType::Bitn => row
            .try_get::<bool, _>(idx)
            .ok()
            .flatten()
            .map(CellValue::Bool)
            .unwrap_or(CellValue::Null),

        // String types
        ColumnType::BigVarChar
        | ColumnType::BigChar
        | ColumnType::NVarchar
        | ColumnType::NChar
        | ColumnType::Text
        | ColumnType::NText => row
            .try_get::<&str, _>(idx)
            .ok()
            .flatten()
            .map(|s| CellValue::String(s.to_string()))
            .unwrap_or(CellValue::Null),

        // Date/Time types
        ColumnType::Datetime | ColumnType::Datetime2 | ColumnType::Datetimen => row
            .try_get::<NaiveDateTime, _>(idx)
            .ok()
            .flatten()
            .map(|dt| CellValue::DateTime(dt.to_string()))
            .unwrap_or(CellValue::Null),
        ColumnType::Datetime4 => row
            .try_get::<NaiveDateTime, _>(idx)
            .ok()
            .flatten()
            .map(|dt| CellValue::DateTime(dt.to_string()))
            .unwrap_or(CellValue::Null),
        ColumnType::DatetimeOffsetn => row
            .try_get::<DateTime<Utc>, _>(idx)
            .ok()
            .flatten()
            .map(|dt| CellValue::DateTime(dt.to_rfc3339()))
            .unwrap_or(CellValue::Null),
        ColumnType::Daten => row
            .try_get::<NaiveDate, _>(idx)
            .ok()
            .flatten()
            .map(|d| CellValue::DateTime(d.to_string()))
            .unwrap_or(CellValue::Null),
        ColumnType::Timen => row
            .try_get::<NaiveTime, _>(idx)
            .ok()
            .flatten()
            .map(|t| CellValue::DateTime(t.to_string()))
            .unwrap_or(CellValue::Null),

        // Binary types
        ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image => row
            .try_get::<&[u8], _>(idx)
            .ok()
            .flatten()
            .map(|b| CellValue::Binary(b.to_vec()))
            .unwrap_or(CellValue::Null),

        // GUID
        ColumnType::Guid => row
            .try_get::<tiberius::Uuid, _>(idx)
            .ok()
            .flatten()
            .map(|u| CellValue::String(u.to_string()))
            .unwrap_or(CellValue::Null),

        // XML - Tiberius returns XML as XmlData type
        ColumnType::Xml => row
            .try_get::<&tiberius::xml::XmlData, _>(idx)
            .ok()
            .flatten()
            .map(|xml| {
                let s = xml.to_owned().into_string();
                CellValue::String(s)
            })
            .unwrap_or(CellValue::Null),

        // Default: try as string
        _ => row
            .try_get::<&str, _>(idx)
            .ok()
            .flatten()
            .map(|s| CellValue::String(s.to_string()))
            .unwrap_or(CellValue::Null),
    }
}

/// Column metadata for a tiberius result column
pub(super) fn column_info(col: &Column) -> ColumnInfo {
    ColumnInfo {
        name: col.name().to_string(),
        data_type: format_sql_data_type(&col.column_type()),
        nullable: true, // Tiberius doesn't easily expose nullability
    }
}

/// Convert Tiberius ColumnType to user-friendly SQL data type string
fn format_sql_data_type(col_type: &ColumnType) -> String {
    match col_type {
        // Integer types
        ColumnType::Int1 => "tinyint".to_string(),
        ColumnType::Int2 => "smallint".to_string(),
        ColumnType::Int4 => "int".to_string(),
        ColumnType::Int8 => "bigint".to_string(),
        ColumnType::Intn => "int".to_string(),

        // Float types
        ColumnType::Float4 => "real".to_string(),
        ColumnType::Float8 => "float".to_string(),
        ColumnType::Floatn => "float".to_string(),

        // Decimal/Numeric types
        ColumnType::Decimaln => "decimal".to_string(),
        ColumnType::Numericn => "numeric".to_string(),
        ColumnType::Money => "money".to_string(),
        ColumnType::Money4 => "smallmoney".to_string(),

        // Boolean types
        ColumnType::Bit => "bit".to_string(),
        ColumnType::Bitn => "bit".to_string(),

        // String types
        ColumnType::BigVarChar => "varchar(max)".to_string(),
        ColumnType::BigChar => "char(max)".to_string(),
        ColumnType::NVarchar => "nvarchar".to_string(),
        ColumnType::NChar => "nchar".to_string(),
        ColumnType::Text => "text".to_string(),
        ColumnType::NText => "ntext".to_string(),

        // Binary types
        ColumnType::BigVarBin => "varbinary(max)".to_string(),
        ColumnType::BigBinary => "binary(max)".to_string(),
        ColumnType::Image => "image".to_string(),

        // Date/Time types
        ColumnType::Datetime => "datetime".to_string(),
        ColumnType::Datetime2 => "datetime2".to_string(),
        ColumnType::Datetimen => "datetime".to_string(),
        ColumnType::Datetime4 => "smalldatetime".to_string(),
        ColumnType::Daten => "date".to_string(),
        ColumnType::Timen => "time".to_string(),
        ColumnType::DatetimeOffsetn => "datetimeoffset".to_string(),

        // Other types
        ColumnType::Guid => "uniqueidentifier".to_string(),
        ColumnType::Xml => "xml".to_string(),
        ColumnType::Null => "null".to_string(),

        // For any other types, fall back to debug format
        _ => format!("{:?}", col_type).to_lowercase(),
    }
}
