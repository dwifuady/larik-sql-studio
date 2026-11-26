use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use mongodb::bson::oid::ObjectId;
use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{Binary, Bson, DateTime as BsonDateTime, Decimal128, Document, Regex, Timestamp};
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use uuid::Uuid;

pub fn format_cell(value: &Bson) -> Option<String> {
    match value {
        Bson::Null | Bson::Undefined => None,
        Bson::Boolean(b) => Some(if *b { "true" } else { "false" }.to_string()),
        Bson::String(s) => Some(s.clone()),
        Bson::Int32(n) => Some(n.to_string()),
        Bson::Int64(n) => Some(n.to_string()),
        Bson::Double(f) => {
            if *f == 0.0 {
                Some("0".to_string())
            } else {
                Some(f.to_string())
            }
        }
        Bson::Decimal128(d) => Some(d.to_string()),
        Bson::ObjectId(oid) => Some(oid.to_hex()),
        Bson::DateTime(dt) => Some(format_bson_datetime(*dt)),
        Bson::Binary(bin) => Some(format_binary(bin)),
        Bson::Timestamp(ts) => Some(format!("Timestamp({}, {})", ts.time, ts.increment)),
        Bson::RegularExpression(re) => Some(format!("/{}/{}", re.pattern, re.options)),
        Bson::JavaScriptCode(code) => Some(code.clone()),
        Bson::JavaScriptCodeWithScope(cws) => Some(cws.code.clone()),
        Bson::Symbol(sym) => Some(sym.clone()),
        Bson::MinKey => Some("MinKey".to_string()),
        Bson::MaxKey => Some("MaxKey".to_string()),
        Bson::DbPointer(_) => Some(format!("{value}")),
        Bson::Array(_) | Bson::Document(_) => {
            let friendly = bson_to_friendly_json(value);
            Some(serde_json::to_string(&friendly).unwrap_or_else(|_| format!("{value}")))
        }
    }
}

pub fn format_bson_datetime(dt: BsonDateTime) -> String {
    let millis = dt.timestamp_millis();
    if let Some(chrono_dt) = DateTime::<Utc>::from_timestamp_millis(millis) {
        chrono_dt.format("%Y-%m-%d %H:%M:%S%.3f Z").to_string()
    } else {
        dt.to_string()
    }
}

pub fn format_binary(bin: &Binary) -> String {
    if bin.bytes.len() == 16 {
        match bin.subtype {
            BinarySubtype::Uuid => {
                if let Ok(uuid) = Uuid::from_slice(&bin.bytes) {
                    return uuid.to_string();
                }
            }
            BinarySubtype::UuidOld => {
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&bin.bytes);
                return Uuid::from_bytes_le(arr).to_string();
            }
            _ => {}
        }
    }
    let mut hex = String::with_capacity(2 + bin.bytes.len() * 2);
    hex.push_str("0x");
    for b in &bin.bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

pub fn bson_to_friendly_json(value: &Bson) -> JsonValue {
    match value {
        Bson::Null | Bson::Undefined => JsonValue::Null,
        Bson::Boolean(b) => JsonValue::Bool(*b),
        Bson::Int32(n) => JsonValue::Number(JsonNumber::from(*n)),
        Bson::Int64(n) => JsonValue::Number(JsonNumber::from(*n)),
        Bson::Double(f) => JsonNumber::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or_else(|| JsonValue::String(f.to_string())),
        Bson::Decimal128(d) => JsonValue::String(d.to_string()),
        Bson::String(s) => JsonValue::String(s.clone()),
        Bson::ObjectId(oid) => JsonValue::String(oid.to_hex()),
        Bson::DateTime(dt) => JsonValue::String(format_bson_datetime(*dt)),
        Bson::Binary(bin) => JsonValue::String(format_binary(bin)),
        Bson::Timestamp(ts) => JsonValue::String(format!("Timestamp({}, {})", ts.time, ts.increment)),
        Bson::RegularExpression(re) => JsonValue::String(format!("/{}/{}", re.pattern, re.options)),
        Bson::JavaScriptCode(code) => JsonValue::String(code.clone()),
        Bson::JavaScriptCodeWithScope(cws) => JsonValue::String(cws.code.clone()),
        Bson::Symbol(sym) => JsonValue::String(sym.clone()),
        Bson::MinKey => JsonValue::String("MinKey".to_string()),
        Bson::MaxKey => JsonValue::String("MaxKey".to_string()),
        Bson::DbPointer(_) => JsonValue::String(format!("{value}")),
        Bson::Array(arr) => JsonValue::Array(arr.iter().map(bson_to_friendly_json).collect()),
        Bson::Document(doc) => {
            let mut map = JsonMap::with_capacity(doc.len());
            for (k, v) in doc {
                map.insert(k.clone(), bson_to_friendly_json(v));
            }
            JsonValue::Object(map)
        }
    }
}

pub fn bson_type_name(value: &Bson) -> &'static str {
    match value {
        Bson::String(_) | Bson::Symbol(_) => "string",
        Bson::Int32(_) => "int32",
        Bson::Int64(_) => "int64",
        Bson::Double(_) => "double",
        Bson::Decimal128(_) => "decimal128",
        Bson::Boolean(_) => "bool",
        Bson::DateTime(_) => "date",
        Bson::ObjectId(_) => "objectId",
        Bson::Binary(bin)
            if bin.bytes.len() == 16
                && matches!(bin.subtype, BinarySubtype::Uuid | BinarySubtype::UuidOld) =>
        {
            "uuid"
        }
        Bson::Binary(_) => "binary",
        Bson::Array(_) => "array",
        Bson::Document(_) => "object",
        Bson::RegularExpression(_) => "regex",
        Bson::Timestamp(_) => "timestamp",
        Bson::Null | Bson::Undefined => "null",
        _ => "bson",
    }
}

pub fn parse_date_string(raw: &str) -> Result<BsonDateTime> {
    let trimmed = raw.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(BsonDateTime::from_millis(dt.timestamp_millis()));
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S%.f Z",
        "%Y-%m-%d %H:%M:%S%.fZ",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
    ] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, fmt) {
            let utc = DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc);
            return Ok(BsonDateTime::from_millis(utc.timestamp_millis()));
        }
    }
    if let Ok(nd) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let ndt = nd.and_hms_opt(0, 0, 0).expect("valid midnight");
        let utc = DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc);
        return Ok(BsonDateTime::from_millis(utc.timestamp_millis()));
    }
    bail!("Invalid date/time literal '{trimmed}'.")
}

pub fn parse_uuid_bson(raw: &str) -> Result<Bson> {
    let uuid = Uuid::parse_str(raw.trim())
        .with_context(|| format!("Invalid UUID literal '{raw}'."))?;
    Ok(Bson::Binary(Binary {
        subtype: BinarySubtype::Uuid,
        bytes: uuid.as_bytes().to_vec(),
    }))
}

pub fn parse_object_id_bson(raw: &str) -> Result<Bson> {
    let oid = ObjectId::parse_str(raw.trim())
        .with_context(|| format!("Invalid ObjectId literal '{raw}'."))?;
    Ok(Bson::ObjectId(oid))
}

pub fn parse_mongosh_value(raw: &str) -> Result<Bson> {
    let mut parser = MongoshParser::new(raw);
    let value = parser.parse_value()?;
    parser.skip_ws_and_comments()?;
    if !parser.is_eof() {
        bail!(
            "Unexpected trailing characters in MongoDB shell argument near '{}'.",
            parser.peek_snippet()
        );
    }
    Ok(value)
}

pub fn parse_mongosh_document(raw: &str) -> Result<Document> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Document::new());
    }
    match parse_mongosh_value(trimmed)? {
        Bson::Document(doc) => Ok(doc),
        other => bail!("Expected a MongoDB document {{ ... }}, got {other}."),
    }
}

pub fn parse_mongosh_document_array(raw: &str) -> Result<Vec<Document>> {
    match parse_mongosh_value(raw.trim())? {
        Bson::Array(items) => {
            let mut docs = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Bson::Document(d) => docs.push(d),
                    other => bail!("Expected document element in array, got {other}."),
                }
            }
            Ok(docs)
        }
        other => bail!("Expected an array of documents [ ... ], got {other}."),
    }
}

struct MongoshParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> MongoshParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<u8> {
        self.bytes.get(self.pos + 1).copied()
    }

    fn peek_snippet(&self) -> &str {
        let end = (self.pos + 24).min(self.src.len());
        let mut safe_end = end;
        while safe_end < self.src.len() && !self.src.is_char_boundary(safe_end) {
            safe_end += 1;
        }
        &self.src[self.pos..safe_end]
    }

    fn skip_ws_and_comments(&mut self) -> Result<()> {
        loop {
            while let Some(b) = self.peek() {
                if b.is_ascii_whitespace() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.peek() == Some(b'/') && self.peek_next() == Some(b'/') {
                self.pos += 2;
                while let Some(b) = self.peek() {
                    self.pos += 1;
                    if b == b'\n' {
                        break;
                    }
                }
                continue;
            }
            if self.peek() == Some(b'/') && self.peek_next() == Some(b'*') {
                self.pos += 2;
                let mut closed = false;
                while self.pos + 1 < self.bytes.len() {
                    if self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/' {
                        self.pos += 2;
                        closed = true;
                        break;
                    }
                    self.pos += 1;
                }
                if !closed {
                    bail!("Unterminated block comment in MongoDB shell argument.");
                }
                continue;
            }
            break;
        }
        Ok(())
    }

    fn parse_value(&mut self) -> Result<Bson> {
        self.skip_ws_and_comments()?;
        let Some(b) = self.peek() else {
            bail!("Unexpected end of input while parsing MongoDB value.");
        };

        match b {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' | b'\'' => Ok(Bson::String(self.parse_string()?)),
            b'/' => self.parse_regex(),
            b'-' | b'+' | b'0'..=b'9' => self.parse_number(),
            _ => self.parse_keyword_or_constructor(),
        }
    }

    fn parse_object(&mut self) -> Result<Bson> {
        self.pos += 1;
        let mut doc = Document::new();
        loop {
            self.skip_ws_and_comments()?;
            if self.peek() == Some(b'}') {
                self.pos += 1;
                break;
            }
            let key = match self.peek() {
                Some(b'"') | Some(b'\'') => self.parse_string()?,
                Some(_) => self.parse_bare_key()?,
                None => bail!("Unterminated object '{{' in MongoDB shell argument."),
            };
            self.skip_ws_and_comments()?;
            if self.peek() != Some(b':') {
                bail!(
                    "Expected ':' after key '{key}' near '{}'.",
                    self.peek_snippet()
                );
            }
            self.pos += 1;
            let value = self.parse_value()?;
            doc.insert(key, value);

            self.skip_ws_and_comments()?;
            if self.peek() == Some(b',') {
                self.pos += 1;
                continue;
            }
            if self.peek() == Some(b'}') {
                self.pos += 1;
                break;
            }
            bail!(
                "Expected ',' or '}}' in object near '{}'.",
                self.peek_snippet()
            );
        }

        convert_extended_json_doc(doc)
    }

    fn parse_array(&mut self) -> Result<Bson> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws_and_comments()?;
            if self.peek() == Some(b']') {
                self.pos += 1;
                break;
            }
            items.push(self.parse_value()?);
            self.skip_ws_and_comments()?;
            if self.peek() == Some(b',') {
                self.pos += 1;
                continue;
            }
            if self.peek() == Some(b']') {
                self.pos += 1;
                break;
            }
            bail!(
                "Expected ',' or ']' in array near '{}'.",
                self.peek_snippet()
            );
        }
        Ok(Bson::Array(items))
    }

    fn parse_bare_key(&mut self) -> Result<String> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'.' | b'-') {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            bail!(
                "Expected object key near '{}'.",
                self.peek_snippet()
            );
        }
        Ok(self.src[start..self.pos].to_string())
    }

    fn parse_string(&mut self) -> Result<String> {
        let quote = self.bytes[self.pos];
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'\\' {
                self.pos += 1;
                if self.pos >= self.bytes.len() {
                    bail!("Unterminated escape sequence in string literal.");
                }
                let esc = self.bytes[self.pos];
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\'' => out.push('\''),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if self.pos + 4 > self.bytes.len() {
                            bail!("Incomplete \\uXXXX escape in string literal.");
                        }
                        let hex = &self.src[self.pos..self.pos + 4];
                        let code = u16::from_str_radix(hex, 16)
                            .with_context(|| format!("Invalid unicode escape \\u{hex}."))?;
                        self.pos += 4;
                        if let Some(ch) = char::from_u32(code as u32) {
                            out.push(ch);
                        }
                    }
                    other => out.push(other as char),
                }
                continue;
            }
            if b == quote {
                self.pos += 1;
                return Ok(out);
            }
            let ch = self.src[self.pos..].chars().next().unwrap();
            out.push(ch);
            self.pos += ch.len_utf8();
        }
        bail!("Unterminated string literal in MongoDB shell argument.")
    }

    fn parse_regex(&mut self) -> Result<Bson> {
        self.pos += 1;
        let mut pattern = String::new();
        let mut in_class = false;
        let mut closed = false;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'\\' && self.pos + 1 < self.bytes.len() {
                pattern.push('\\');
                let next = self.src[self.pos + 1..].chars().next().unwrap();
                pattern.push(next);
                self.pos += 1 + next.len_utf8();
                continue;
            }
            if b == b'[' {
                in_class = true;
            } else if b == b']' {
                in_class = false;
            } else if b == b'/' && !in_class {
                self.pos += 1;
                closed = true;
                break;
            }
            let ch = self.src[self.pos..].chars().next().unwrap();
            pattern.push(ch);
            self.pos += ch.len_utf8();
        }
        if !closed {
            bail!("Unterminated regular expression literal.");
        }
        let mut options = String::new();
        while let Some(b) = self.peek() {
            if b.is_ascii_alphabetic() {
                options.push(b as char);
                self.pos += 1;
            } else {
                break;
            }
        }
        Ok(Bson::RegularExpression(Regex { pattern, options }))
    }

    fn parse_number(&mut self) -> Result<Bson> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'-') | Some(b'+')) {
            self.pos += 1;
        }
        let mut is_float = false;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else if matches!(b, b'.' | b'e' | b'E') {
                is_float = true;
                self.pos += 1;
                if matches!(b, b'e' | b'E') && matches!(self.peek(), Some(b'+') | Some(b'-')) {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        let raw = &self.src[start..self.pos];
        if is_float {
            let v: f64 = raw
                .parse()
                .with_context(|| format!("Invalid floating-point literal '{raw}'."))?;
            Ok(Bson::Double(v))
        } else if let Ok(v) = raw.parse::<i32>() {
            Ok(Bson::Int32(v))
        } else if let Ok(v) = raw.parse::<i64>() {
            Ok(Bson::Int64(v))
        } else {
            let v: f64 = raw
                .parse()
                .with_context(|| format!("Invalid numeric literal '{raw}'."))?;
            Ok(Bson::Double(v))
        }
    }

    fn parse_ident(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$') {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.src[start..self.pos].to_string()
    }

    fn parse_keyword_or_constructor(&mut self) -> Result<Bson> {
        let mut word = self.parse_ident();
        if word == "new" {
            self.skip_ws_and_comments()?;
            word = self.parse_ident();
        }
        if word.is_empty() {
            bail!(
                "Unexpected token near '{}'.",
                self.peek_snippet()
            );
        }

        match word.as_str() {
            "true" => return Ok(Bson::Boolean(true)),
            "false" => return Ok(Bson::Boolean(false)),
            "null" | "undefined" => return Ok(Bson::Null),
            _ => {}
        }

        self.skip_ws_and_comments()?;
        if self.peek() != Some(b'(') {
            bail!("Unexpected identifier '{word}' in MongoDB shell argument.");
        }
        self.pos += 1;
        let mut args = Vec::new();
        loop {
            self.skip_ws_and_comments()?;
            if self.peek() == Some(b')') {
                self.pos += 1;
                break;
            }
            args.push(self.parse_value()?);
            self.skip_ws_and_comments()?;
            if self.peek() == Some(b',') {
                self.pos += 1;
                continue;
            }
            if self.peek() == Some(b')') {
                self.pos += 1;
                break;
            }
            bail!("Expected ',' or ')' in {word}(...) call.");
        }

        eval_bson_constructor(&word, &args)
    }
}

pub fn eval_bson_constructor(name: &str, args: &[Bson]) -> Result<Bson> {
    if name.eq_ignore_ascii_case("ObjectId") {
        match args {
            [] => Ok(Bson::ObjectId(ObjectId::new())),
            [Bson::String(s)] => parse_object_id_bson(s),
            _ => bail!("ObjectId() expects zero arguments or a 24-character hex string."),
        }
    } else if name.eq_ignore_ascii_case("ISODate")
        || name.eq_ignore_ascii_case("Date")
        || name.eq_ignore_ascii_case("DateTime")
    {
        match args {
            [] => Ok(Bson::DateTime(BsonDateTime::now())),
            [Bson::String(s)] => Ok(Bson::DateTime(parse_date_string(s)?)),
            [Bson::Int32(ms)] => Ok(Bson::DateTime(BsonDateTime::from_millis(*ms as i64))),
            [Bson::Int64(ms)] => Ok(Bson::DateTime(BsonDateTime::from_millis(*ms))),
            _ => bail!("{name}() expects an ISO-8601 date string or epoch milliseconds."),
        }
    } else if name.eq_ignore_ascii_case("UUID") || name.eq_ignore_ascii_case("GUID") {
        match args {
            [] => Ok(Bson::Binary(Binary {
                subtype: BinarySubtype::Uuid,
                bytes: Uuid::new_v4().as_bytes().to_vec(),
            })),
            [Bson::String(s)] => parse_uuid_bson(s),
            _ => bail!("{name}() expects a UUID string."),
        }
    } else if name.eq_ignore_ascii_case("NumberInt") {
        match args {
            [Bson::Int32(n)] => Ok(Bson::Int32(*n)),
            [Bson::Int64(n)] => Ok(Bson::Int32(*n as i32)),
            [Bson::Double(f)] => Ok(Bson::Int32(*f as i32)),
            [Bson::String(s)] => Ok(Bson::Int32(
                s.trim()
                    .parse::<i32>()
                    .with_context(|| format!("Invalid NumberInt('{s}')."))?,
            )),
            _ => bail!("NumberInt() expects an integer argument."),
        }
    } else if name.eq_ignore_ascii_case("NumberLong") {
        match args {
            [Bson::Int32(n)] => Ok(Bson::Int64(*n as i64)),
            [Bson::Int64(n)] => Ok(Bson::Int64(*n)),
            [Bson::Double(f)] => Ok(Bson::Int64(*f as i64)),
            [Bson::String(s)] => Ok(Bson::Int64(
                s.trim()
                    .parse::<i64>()
                    .with_context(|| format!("Invalid NumberLong('{s}')."))?,
            )),
            _ => bail!("NumberLong() expects a 64-bit integer argument."),
        }
    } else if name.eq_ignore_ascii_case("NumberDecimal") {
        let raw = match args {
            [Bson::String(s)] => s.clone(),
            [Bson::Int32(n)] => n.to_string(),
            [Bson::Int64(n)] => n.to_string(),
            [Bson::Double(f)] => f.to_string(),
            _ => bail!("NumberDecimal() expects a decimal string or number."),
        };
        let dec = Decimal128::from_str(raw.trim())
            .map_err(|e| anyhow!("Invalid NumberDecimal('{raw}'): {e}"))?;
        Ok(Bson::Decimal128(dec))
    } else if name.eq_ignore_ascii_case("Timestamp") {
        let to_u32 = |v: &Bson| -> Result<u32> {
            match v {
                Bson::Int32(n) if *n >= 0 => Ok(*n as u32),
                Bson::Int64(n) if *n >= 0 => Ok(*n as u32),
                _ => bail!("Timestamp() expects non-negative integer arguments."),
            }
        };
        match args {
            [t, i] => Ok(Bson::Timestamp(Timestamp {
                time: to_u32(t)?,
                increment: to_u32(i)?,
            })),
            _ => bail!("Timestamp(time, increment) expects 2 arguments."),
        }
    } else {
        bail!("Unsupported MongoDB constructor '{name}(...)'.");
    }
}

fn convert_extended_json_doc(doc: Document) -> Result<Bson> {
    if doc.len() == 1 {
        if let Some(Bson::String(hex)) = doc.get("$oid") {
            return parse_object_id_bson(hex);
        }
        if let Some(Bson::String(s)) = doc.get("$uuid") {
            return parse_uuid_bson(s);
        }
        if let Some(date_val) = doc.get("$date") {
            match date_val {
                Bson::String(s) => return Ok(Bson::DateTime(parse_date_string(s)?)),
                Bson::Int64(ms) => return Ok(Bson::DateTime(BsonDateTime::from_millis(*ms))),
                Bson::Int32(ms) => {
                    return Ok(Bson::DateTime(BsonDateTime::from_millis(*ms as i64)));
                }
                Bson::Document(inner) => {
                    if let Some(Bson::String(ms_str)) = inner.get("$numberLong") {
                        if let Ok(ms) = ms_str.parse::<i64>() {
                            return Ok(Bson::DateTime(BsonDateTime::from_millis(ms)));
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(Bson::String(s)) = doc.get("$numberInt") {
            if let Ok(n) = s.parse::<i32>() {
                return Ok(Bson::Int32(n));
            }
        }
        if let Some(Bson::String(s)) = doc.get("$numberLong") {
            if let Ok(n) = s.parse::<i64>() {
                return Ok(Bson::Int64(n));
            }
        }
        if let Some(Bson::String(s)) = doc.get("$numberDecimal") {
            if let Ok(d) = Decimal128::from_str(s) {
                return Ok(Bson::Decimal128(d));
            }
        }
        if let Some(Bson::Document(re_doc)) = doc.get("$regularExpression") {
            if let Some(Bson::String(pattern)) = re_doc.get("pattern") {
                let options = re_doc
                    .get_str("options")
                    .unwrap_or_default()
                    .to_string();
                return Ok(Bson::RegularExpression(Regex {
                    pattern: pattern.clone(),
                    options,
                }));
            }
        }
    }
    Ok(Bson::Document(doc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn formats_primitives_dates_object_ids_and_uuids() {
        assert_eq!(format_cell(&Bson::Null), None);
        assert_eq!(format_cell(&Bson::Boolean(true)).as_deref(), Some("true"));
        assert_eq!(format_cell(&Bson::Boolean(false)).as_deref(), Some("false"));
        assert_eq!(format_cell(&Bson::Int64(42)).as_deref(), Some("42"));
        assert_eq!(
            format_cell(&Bson::String("hello".into())).as_deref(),
            Some("hello")
        );

        let oid = ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap();
        assert_eq!(
            format_cell(&Bson::ObjectId(oid)).as_deref(),
            Some("507f1f77bcf86cd799439011")
        );

        let dt = parse_date_string("2025-03-14T09:26:53.123Z").unwrap();
        assert_eq!(
            format_cell(&Bson::DateTime(dt)).as_deref(),
            Some("2025-03-14 09:26:53.123 Z")
        );

        let uuid = Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap();
        let b4 = Bson::Binary(Binary {
            subtype: BinarySubtype::Uuid,
            bytes: uuid.as_bytes().to_vec(),
        });
        assert_eq!(
            format_cell(&b4).as_deref(),
            Some("00112233-4455-6677-8899-aabbccddeeff")
        );
        let b3 = Bson::Binary(Binary {
            subtype: BinarySubtype::UuidOld,
            bytes: uuid.to_bytes_le().to_vec(),
        });
        assert_eq!(
            format_cell(&b3).as_deref(),
            Some("00112233-4455-6677-8899-aabbccddeeff")
        );

        let nested = Bson::Document(doc! { "_id": oid, "ok": true });
        assert_eq!(
            format_cell(&nested).as_deref(),
            Some(r#"{"_id":"507f1f77bcf86cd799439011","ok":true}"#)
        );
    }

    #[test]
    fn parses_relaxed_mongosh_documents_and_helpers() {
        let doc = parse_mongosh_document(
            "{ status: 'paid', count: { $gte: 5 }, id: ObjectId('507f1f77bcf86cd799439011'), at: ISODate('2025-01-01T00:00:00Z'), tag: /foo/i, }",
        )
        .unwrap();
        assert_eq!(doc.get_str("status").unwrap(), "paid");
        assert_eq!(doc.get_document("count").unwrap().get_i32("$gte").unwrap(), 5);
        assert_eq!(
            doc.get_object_id("id").unwrap().to_hex(),
            "507f1f77bcf86cd799439011"
        );
        assert!(matches!(doc.get("at"), Some(Bson::DateTime(_))));
        assert!(matches!(doc.get("tag"), Some(Bson::RegularExpression(_))));
    }
}
