// tokio-postgres -> driver-neutral value conversion
// Maps PostgreSQL column values onto `CellValue` / `ColumnInfo`, mirroring the
// conventions of `db/mssql/values.rs` (dates as `DateTime` strings, binary as
// `CellValue::Binary`, numeric as `Float` when exact).
//
// Rows arrive in the extended protocol's *binary* format. Every cell is read
// as raw bytes and decoded here, so a type we can't decode degrades to a
// `Binary` cell — never to a misleading NULL or to binary bytes shown as text.

use std::error::Error;
use std::fmt::Write as _;

use crate::db::query::{CellValue, ColumnInfo};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use fallible_iterator::FallibleIterator;
use postgres_protocol::types as wire;
use tokio_postgres::types::{FromSql, Kind, Type};
use tokio_postgres::{Column, Row};

type DecodeResult<T> = Result<T, Box<dyn Error + Sync + Send>>;

/// Decode the cell at column `idx` of `row`, whose PostgreSQL type is `ty`.
pub(super) fn cell_from_row(row: &Row, idx: usize, ty: &Type) -> CellValue {
    match row.try_get::<_, RawValue>(idx) {
        Ok(RawValue(raw)) => decode(ty, raw),
        Err(_) => CellValue::Null,
    }
}

/// Raw wire bytes of a cell (`None` = SQL NULL); accepts every type.
struct RawValue<'a>(Option<&'a [u8]>);

impl<'a> FromSql<'a> for RawValue<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> DecodeResult<Self> {
        Ok(RawValue(Some(raw)))
    }

    fn from_sql_null(_: &Type) -> DecodeResult<Self> {
        Ok(RawValue(None))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

/// Decode one binary-format value. Undecodable values become `Binary` cells.
fn decode(ty: &Type, raw: Option<&[u8]>) -> CellValue {
    match raw {
        None => CellValue::Null,
        Some(raw) => decode_value(ty, raw).unwrap_or_else(|_| CellValue::Binary(raw.to_vec())),
    }
}

fn decode_value(ty: &Type, raw: &[u8]) -> DecodeResult<CellValue> {
    Ok(match *ty {
        Type::BOOL => CellValue::Bool(bool::from_sql(ty, raw)?),
        Type::INT2 => CellValue::Int(i64::from(i16::from_sql(ty, raw)?)),
        Type::INT4 => CellValue::Int(i64::from(i32::from_sql(ty, raw)?)),
        Type::INT8 => CellValue::Int(i64::from_sql(ty, raw)?),
        Type::OID => CellValue::Int(i64::from(u32::from_sql(ty, raw)?)),
        // Internal single-byte "char" (e.g. pg_class.relkind)
        Type::CHAR => CellValue::String(char::from(i8::from_sql(ty, raw)? as u8).to_string()),
        Type::FLOAT4 => CellValue::Float(f64::from(f32::from_sql(ty, raw)?)),
        Type::FLOAT8 => CellValue::Float(f64::from_sql(ty, raw)?),
        Type::NUMERIC => numeric_cell(numeric_text(raw)?),
        // Fixed-point cents (assumes the usual 2-digit lc_monetary)
        Type::MONEY => CellValue::Float(wire::int8_from_sql(raw)? as f64 / 100.0),

        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN | Type::XML => {
            CellValue::String(std::str::from_utf8(raw)?.to_string())
        }

        Type::UUID => CellValue::String(uuid::Uuid::from_sql(ty, raw)?.to_string()),
        Type::JSON | Type::JSONB => {
            CellValue::String(serde_json::Value::from_sql(ty, raw)?.to_string())
        }

        Type::DATE => CellValue::DateTime(match wire::date_from_sql(raw)? {
            i32::MAX => "infinity".to_string(),
            i32::MIN => "-infinity".to_string(),
            _ => NaiveDate::from_sql(ty, raw)?.to_string(),
        }),
        Type::TIME => CellValue::DateTime(NaiveTime::from_sql(ty, raw)?.to_string()),
        Type::TIMETZ => CellValue::DateTime(timetz_text(raw)?),
        Type::TIMESTAMP => CellValue::DateTime(match wire::timestamp_from_sql(raw)? {
            i64::MAX => "infinity".to_string(),
            i64::MIN => "-infinity".to_string(),
            _ => NaiveDateTime::from_sql(ty, raw)?.to_string(),
        }),
        Type::TIMESTAMPTZ => CellValue::DateTime(match wire::timestamp_from_sql(raw)? {
            i64::MAX => "infinity".to_string(),
            i64::MIN => "-infinity".to_string(),
            _ => DateTime::<Utc>::from_sql(ty, raw)?.to_rfc3339(),
        }),
        Type::INTERVAL => CellValue::String(interval_text(raw)?),

        Type::INET | Type::CIDR => CellValue::String(inet_text(raw, *ty == Type::CIDR)?),
        Type::BIT | Type::VARBIT => CellValue::String(varbit_text(raw)?),

        Type::VOID => CellValue::String(String::new()),
        Type::BYTEA => CellValue::Binary(raw.to_vec()),

        _ => match ty.kind() {
            Kind::Array(elem) => CellValue::String(array_text(elem, raw)?),
            Kind::Domain(base) => decode_value(base, raw)?,
            // Enum labels are sent as text even in binary format.
            Kind::Enum(_) => CellValue::String(std::str::from_utf8(raw)?.to_string()),
            // Custom types: many extensions (citext, ...) send text; anything
            // with control bytes is genuinely binary.
            _ => printable_text(raw)
                .map(CellValue::String)
                .unwrap_or_else(|| CellValue::Binary(raw.to_vec())),
        },
    })
}

/// UTF-8 text without control characters (other than common whitespace).
fn printable_text(raw: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(raw).ok()?;
    s.chars()
        .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .then(|| s.to_string())
}

/// Exact decimal text of a binary NUMERIC (base-10000 digits), including
/// `NaN` / `Infinity` and values beyond any fixed-width decimal type.
fn numeric_text(raw: &[u8]) -> DecodeResult<String> {
    let header = |offset: usize| -> DecodeResult<u16> {
        raw.get(offset..offset + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .ok_or_else(|| "invalid numeric".into())
    };
    let ndigits = usize::from(header(0)?);
    let weight = i32::from(header(2)? as i16);
    let sign = header(4)?;
    let dscale = usize::from(header(6)?);
    match sign {
        0xC000 => return Ok("NaN".to_string()),
        0xD000 => return Ok("Infinity".to_string()),
        0xF000 => return Ok("-Infinity".to_string()),
        0x0000 | 0x4000 => {}
        _ => return Err("invalid numeric sign".into()),
    }
    let digits = (0..ndigits)
        .map(|k| header(8 + 2 * k).map(|d| i32::from(d as i16)))
        .collect::<DecodeResult<Vec<i32>>>()?;
    // Group `i` holds the base-10000 digit for power (weight - i).
    let digit = |i: i32| usize::try_from(i).ok().and_then(|i| digits.get(i)).copied().unwrap_or(0);

    let mut out = String::new();
    if sign == 0x4000 {
        out.push('-');
    }
    if weight < 0 {
        out.push('0');
    } else {
        for i in 0..=weight {
            let _ = if i == 0 {
                write!(out, "{}", digit(i))
            } else {
                write!(out, "{:04}", digit(i))
            };
        }
    }
    if dscale > 0 {
        let mut frac = String::new();
        let mut k = 1;
        while frac.len() < dscale {
            let _ = write!(frac, "{:04}", digit(weight + k));
            k += 1;
        }
        frac.truncate(dscale);
        out.push('.');
        out.push_str(&frac);
    }
    Ok(out)
}

/// `Float` when the value survives the round trip through f64 (≤ 15
/// significant digits), otherwise the exact decimal text.
fn numeric_cell(text: String) -> CellValue {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    let mantissa = text
        .split_once('.')
        .map(|_| digits.trim_end_matches('0'))
        .unwrap_or(&digits)
        .trim_start_matches('0');
    if mantissa.len() <= 15 {
        if let Ok(v) = text.parse::<f64>() {
            if v.is_finite() {
                return CellValue::Float(v);
            }
        }
    }
    CellValue::String(text)
}

/// `HH:MM:SS[.ffffff]` from microseconds (no sign).
fn clock_text(micros: u64) -> String {
    let secs = micros / 1_000_000;
    let frac = micros % 1_000_000;
    let mut out = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    if frac != 0 {
        let fraction = format!("{frac:06}");
        out.push('.');
        out.push_str(fraction.trim_end_matches('0'));
    }
    out
}

/// TIMETZ: 8-byte time of day (µs) + 4-byte zone offset in seconds *west* of UTC.
fn timetz_text(raw: &[u8]) -> DecodeResult<String> {
    if raw.len() != 12 {
        return Err("invalid timetz".into());
    }
    let micros = wire::time_from_sql(&raw[..8])?;
    let west = wire::int4_from_sql(&raw[8..])?;
    let east = -i64::from(west);
    let abs = east.unsigned_abs();
    let mut out = clock_text(micros.unsigned_abs());
    let _ = write!(out, "{}{:02}", if east < 0 { '-' } else { '+' }, abs / 3600);
    if abs % 3600 != 0 {
        let _ = write!(out, ":{:02}", abs / 60 % 60);
        if abs % 60 != 0 {
            let _ = write!(out, ":{:02}", abs % 60);
        }
    }
    Ok(out)
}

/// INTERVAL (µs, days, months) in PostgreSQL's default `postgres` style:
/// `1 year 2 mons 3 days 04:05:06.5`.
fn interval_text(raw: &[u8]) -> DecodeResult<String> {
    if raw.len() != 16 {
        return Err("invalid interval".into());
    }
    let micros = wire::int8_from_sql(&raw[..8])?;
    let days = wire::int4_from_sql(&raw[8..12])?;
    let months = wire::int4_from_sql(&raw[12..])?;

    let mut parts: Vec<String> = Vec::new();
    let mut any_negative = false;
    for (value, unit) in [(months / 12, "year"), (months % 12, "mon"), (days, "day")] {
        if value != 0 {
            any_negative |= value < 0;
            parts.push(format!("{value} {unit}{}", if value == 1 { "" } else { "s" }));
        }
    }
    if micros != 0 || parts.is_empty() {
        let sign = if micros < 0 {
            "-"
        } else if any_negative {
            "+"
        } else {
            ""
        };
        parts.push(format!("{sign}{}", clock_text(micros.unsigned_abs())));
    }
    Ok(parts.join(" "))
}

/// INET / CIDR. `inet` omits a full-length netmask, `cidr` always shows it.
fn inet_text(raw: &[u8], is_cidr: bool) -> DecodeResult<String> {
    let inet = wire::inet_from_sql(raw)?;
    let addr = inet.addr();
    let full = if addr.is_ipv4() { 32 } else { 128 };
    Ok(if is_cidr || inet.netmask() != full {
        format!("{}/{}", addr, inet.netmask())
    } else {
        addr.to_string()
    })
}

/// BIT / VARBIT as a `0101...` string.
fn varbit_text(raw: &[u8]) -> DecodeResult<String> {
    let bits = wire::varbit_from_sql(raw)?;
    let bytes = bits.bytes();
    Ok((0..bits.len())
        .map(|i| {
            let set = bytes.get(i / 8).is_some_and(|b| b & (0x80 >> (i % 8)) != 0);
            if set {
                '1'
            } else {
                '0'
            }
        })
        .collect())
}

/// Render an array as a PostgreSQL literal (`{1,NULL,3}`, `{{a,b},{c,d}}`,
/// `[0:1]={x,y}`), decoding each element with the scalar decoder.
fn array_text(elem: &Type, raw: &[u8]) -> DecodeResult<String> {
    let array = wire::array_from_sql(raw)?;
    let dims: Vec<(i32, i32)> = array
        .dimensions()
        .map(|d| Ok((d.len, d.lower_bound)))
        .collect()?;
    let values: Vec<Option<String>> = array
        .values()
        .map(|v| Ok(element_text(decode(elem, v))))
        .collect()?;

    let mut out = String::new();
    if dims.is_empty() {
        out.push_str("{}");
        return Ok(out);
    }
    if dims.iter().any(|(_, lower)| *lower != 1) {
        for (len, lower) in &dims {
            let _ = write!(out, "[{}:{}]", lower, lower + len - 1);
        }
        out.push('=');
    }
    let lens: Vec<usize> = dims
        .iter()
        .map(|(len, _)| usize::try_from(*len).unwrap_or(0))
        .collect();
    let mut values = values.into_iter();
    render_array_level(&lens, &mut values, &mut out);
    Ok(out)
}

fn render_array_level(
    lens: &[usize],
    values: &mut impl Iterator<Item = Option<String>>,
    out: &mut String,
) {
    let Some((&len, inner)) = lens.split_first() else {
        return;
    };
    out.push('{');
    for k in 0..len {
        if k > 0 {
            out.push(',');
        }
        if inner.is_empty() {
            match values.next().flatten() {
                Some(text) => out.push_str(&quote_array_elem(&text)),
                None => out.push_str("NULL"),
            }
        } else {
            render_array_level(inner, values, out);
        }
    }
    out.push('}');
}

/// Element text as `array_out` would print it (`None` = NULL element).
fn element_text(cell: CellValue) -> Option<String> {
    match cell {
        CellValue::Null => None,
        CellValue::Bool(b) => Some(if b { "t" } else { "f" }.to_string()),
        CellValue::Int(v) => Some(v.to_string()),
        CellValue::Float(v) => Some(v.to_string()),
        CellValue::String(s) | CellValue::DateTime(s) => Some(s),
        CellValue::Binary(bytes) => {
            let mut hex = String::from("\\x");
            for b in bytes {
                let _ = write!(hex, "{b:02x}");
            }
            Some(hex)
        }
    }
}

/// Quote an array element the way `array_out` does: when empty, spelled
/// `NULL`, or containing braces, commas, quotes, backslashes or whitespace.
fn quote_array_elem(text: &str) -> String {
    let needs_quotes = text.is_empty()
        || text.eq_ignore_ascii_case("NULL")
        || text
            .chars()
            .any(|c| matches!(c, '{' | '}' | ',' | '"' | '\\') || c.is_whitespace());
    if needs_quotes {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        text.to_string()
    }
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
        Type::TIMETZ => "time with time zone".into(),
        Type::TIMESTAMP => "timestamp without time zone".into(),
        Type::TIMESTAMPTZ => "timestamp with time zone".into(),
        Type::INTERVAL => "interval".into(),
        Type::INET => "inet".into(),
        Type::CIDR => "cidr".into(),
        Type::BIT => "bit".into(),
        Type::VARBIT => "bit varying".into(),
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

    /// Binary NUMERIC: header (ndigits, weight, sign, dscale) + base-10000 digits.
    fn numeric(weight: i16, sign: u16, dscale: u16, digits: &[i16]) -> Vec<u8> {
        let mut raw = Vec::new();
        raw.extend((digits.len() as u16).to_be_bytes());
        raw.extend(weight.to_be_bytes());
        raw.extend(sign.to_be_bytes());
        raw.extend(dscale.to_be_bytes());
        for d in digits {
            raw.extend(d.to_be_bytes());
        }
        raw
    }

    #[test]
    fn formats_standard_types() {
        assert_eq!(format_type(&Type::INT4), "integer");
        assert_eq!(format_type(&Type::TIMESTAMPTZ), "timestamp with time zone");
        assert_eq!(format_type(&Type::VARCHAR), "character varying");
        assert_eq!(format_type(&Type::INTERVAL), "interval");
    }

    #[test]
    fn formats_arrays() {
        assert_eq!(format_type(&Type::TEXT_ARRAY), "text[]");
        assert_eq!(format_type(&Type::INT4_ARRAY), "integer[]");
    }

    #[test]
    fn numeric_is_exact() {
        // 12.345 -> groups [12, 3450], weight 0, dscale 3
        assert_eq!(numeric_text(&numeric(0, 0, 3, &[12, 3450])).unwrap(), "12.345");
        // -0.0001 -> groups [1], weight -1
        assert_eq!(numeric_text(&numeric(-1, 0x4000, 4, &[1])).unwrap(), "-0.0001");
        // 10^30 -> group [100] at weight 7 (100 * 10000^7)
        assert_eq!(
            numeric_text(&numeric(7, 0, 0, &[100])).unwrap(),
            "1000000000000000000000000000000"
        );
        assert_eq!(numeric_text(&numeric(0, 0xC000, 0, &[])).unwrap(), "NaN");
        assert_eq!(numeric_text(&numeric(0, 0xD000, 0, &[])).unwrap(), "Infinity");
    }

    #[test]
    fn numeric_cells_keep_precision() {
        assert_eq!(numeric_cell("12.345".into()), CellValue::Float(12.345));
        assert_eq!(numeric_cell("12.3450000".into()), CellValue::Float(12.345));
        assert_eq!(
            numeric_cell("12345678901234567890.1234567890".into()),
            CellValue::String("12345678901234567890.1234567890".into())
        );
        assert_eq!(numeric_cell("NaN".into()), CellValue::String("NaN".into()));
    }

    #[test]
    fn intervals_use_postgres_style() {
        let raw = |micros: i64, days: i32, months: i32| {
            let mut v = micros.to_be_bytes().to_vec();
            v.extend(days.to_be_bytes());
            v.extend(months.to_be_bytes());
            v
        };
        assert_eq!(interval_text(&raw(0, 1, 0)).unwrap(), "1 day");
        assert_eq!(
            interval_text(&raw(14_706_500_000, 3, 14)).unwrap(),
            "1 year 2 mons 3 days 04:05:06.5"
        );
        assert_eq!(interval_text(&raw(0, 0, 0)).unwrap(), "00:00:00");
        assert_eq!(interval_text(&raw(3_600_000_000, -1, 0)).unwrap(), "-1 days +01:00:00");
        assert_eq!(interval_text(&raw(-3_600_000_000, 0, 0)).unwrap(), "-01:00:00");
    }

    #[test]
    fn timetz_shows_offset() {
        let mut raw = (36_672_000_000i64).to_be_bytes().to_vec(); // 10:11:12
        raw.extend((-7200i32).to_be_bytes()); // 2h east of UTC
        assert_eq!(timetz_text(&raw).unwrap(), "10:11:12+02");
    }

    #[test]
    fn inet_and_cidr_masks() {
        let v4 = |mask: u8, is_cidr: u8| vec![2, mask, is_cidr, 4, 10, 0, 0, 0];
        assert_eq!(inet_text(&v4(8, 1), true).unwrap(), "10.0.0.0/8");
        assert_eq!(inet_text(&v4(8, 0), false).unwrap(), "10.0.0.0/8");
        assert_eq!(inet_text(&v4(32, 0), false).unwrap(), "10.0.0.0");
    }

    #[test]
    fn money_and_varbit_decode() {
        assert_eq!(
            decode(&Type::MONEY, Some(&12345i64.to_be_bytes())),
            CellValue::Float(123.45)
        );
        let mut bits = 5i32.to_be_bytes().to_vec();
        bits.push(0b1010_1000);
        assert_eq!(varbit_text(&bits).unwrap(), "10101");
    }

    #[test]
    fn unknown_binary_is_not_shown_as_text() {
        assert_eq!(
            decode(&Type::TS_VECTOR, Some(&[0, 0, 0, 1])),
            CellValue::Binary(vec![0, 0, 0, 1])
        );
        assert_eq!(
            decode(&Type::TS_VECTOR, Some(b"hello")),
            CellValue::String("hello".into())
        );
    }

    #[test]
    fn quotes_array_elements_like_array_out() {
        assert_eq!(quote_array_elem("a"), "a");
        assert_eq!(quote_array_elem("a b"), "\"a b\"");
        assert_eq!(quote_array_elem("NULL"), "\"NULL\"");
        assert_eq!(quote_array_elem(""), "\"\"");
        assert_eq!(quote_array_elem("x\"y"), "\"x\\\"y\"");
    }
}
