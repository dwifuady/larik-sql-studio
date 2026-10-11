// mongo-shell syntax parser
//
// Parses the subset of mongosh syntax the editor accepts:
//   db.<collection>.<method>(args...)[.<cursorMethod>(args...)]...
//   db.getCollection("name").<method>(...),  db["name"].<method>(...)
//   db.getSiblingDB("other").<collection>.<method>(...)
//   db.<databaseMethod>(args...)          e.g. db.runCommand({ ping: 1 })
//   show dbs | show collections,  use <database>
//
// Arguments are JavaScript-style literals (unquoted keys, single/double quotes,
// trailing commas, comments, regex literals) plus the shell helpers ObjectId(),
// ISODate(), new Date(), NumberLong(), ... — parsed into `Value`, never executed.
// No JavaScript engine is involved: anything outside this grammar is a syntax error.

use std::fmt;

/// A literal value from the shell text. Helper calls stay unevaluated as `Call`
/// and are converted to BSON in `convert.rs`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Undefined,
    Bool(bool),
    Int(i64),
    Double(f64),
    Str(String),
    Regex {
        pattern: String,
        flags: String,
    },
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
    /// `Name(args)` or `new Name(args)`, e.g. `ObjectId("...")`
    Call {
        name: String,
        args: Vec<Value>,
    },
}

/// `.name(args)` in a method chain
#[derive(Debug, Clone, PartialEq)]
pub struct MethodCall {
    pub name: String,
    pub args: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShowTarget {
    Databases,
    Collections,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Show(ShowTarget),
    Use(String),
    /// `db.<method>(...)`
    DatabaseCall {
        database: Option<String>,
        call: MethodCall,
    },
    /// `db.<collection>.<method>(...).<chain>...`
    CollectionCall {
        database: Option<String>,
        collection: String,
        call: MethodCall,
        chain: Vec<MethodCall>,
    },
}

/// A parse failure with the byte offset it occurred at
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub offset: usize,
}

impl ParseError {
    fn new(message: impl Into<String>, offset: usize) -> Self {
        Self {
            message: message.into(),
            offset,
        }
    }

    /// Render with a 1-based line/column computed from `source`
    pub fn describe(&self, source: &str) -> String {
        let upto = &source[..self.offset.min(source.len())];
        let line = upto.matches('\n').count() + 1;
        let column = upto.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        format!(
            "Syntax error at line {line}, column {column}: {}",
            self.message
        )
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at offset {})", self.message, self.offset)
    }
}

// ============================================================================
// Lexer
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Int(i64),
    Double(f64),
    Regex { pattern: String, flags: String },
    Punct(char),
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    start: usize,
    end: usize,
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Characters after which a `/` starts a regex literal (there is no division
/// in this grammar, but be strict to give better errors).
fn regex_allowed_after(prev: Option<&Tok>) -> bool {
    matches!(
        prev,
        None | Some(Tok::Punct('(' | ',' | ':' | '[' | '{' | ';'))
    )
}

fn tokenize(src: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let mut tokens: Vec<Token> = Vec::new();
    let mut i = 0usize;
    let byte_at = |idx: usize| chars.get(idx).map_or(src.len(), |&(b, _)| b);

    while i < chars.len() {
        let (start, c) = chars[i];
        let next = chars.get(i + 1).map(|&(_, c)| c);

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // Comments
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i].1 != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            i += 2;
            loop {
                if i >= chars.len() {
                    return Err(ParseError::new("Unterminated block comment", start));
                }
                if chars[i].1 == '*' && chars.get(i + 1).map(|&(_, c)| c) == Some('/') {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // Strings
        if c == '"' || c == '\'' || c == '`' {
            let quote = c;
            let mut value = String::new();
            i += 1;
            loop {
                let Some(&(_, ch)) = chars.get(i) else {
                    return Err(ParseError::new("Unterminated string", start));
                };
                i += 1;
                if ch == quote {
                    break;
                }
                if ch == '\\' {
                    let Some(&(_, esc)) = chars.get(i) else {
                        return Err(ParseError::new("Unterminated string", start));
                    };
                    i += 1;
                    match esc {
                        'n' => value.push('\n'),
                        't' => value.push('\t'),
                        'r' => value.push('\r'),
                        'b' => value.push('\u{8}'),
                        'f' => value.push('\u{c}'),
                        '0' => value.push('\0'),
                        'u' => {
                            let hex: String =
                                chars.iter().skip(i).take(4).map(|&(_, c)| c).collect();
                            let code = u32::from_str_radix(&hex, 16)
                                .ok()
                                .filter(|_| hex.len() == 4)
                                .and_then(char::from_u32)
                                .ok_or_else(|| {
                                    ParseError::new("Invalid \\u escape", byte_at(i - 2))
                                })?;
                            value.push(code);
                            i += 4;
                        }
                        '\n' => {} // line continuation
                        other => value.push(other),
                    }
                    continue;
                }
                if ch == '\n' && quote != '`' {
                    return Err(ParseError::new("Unterminated string", start));
                }
                value.push(ch);
            }
            tokens.push(Token {
                tok: Tok::Str(value),
                start,
                end: byte_at(i),
            });
            continue;
        }

        // Numbers
        if c.is_ascii_digit() || (c == '.' && next.is_some_and(|n| n.is_ascii_digit())) {
            let begin = i;
            let mut is_float = false;
            if c == '0' && matches!(next, Some('x' | 'X')) {
                i += 2;
                while i < chars.len() && chars[i].1.is_ascii_hexdigit() {
                    i += 1;
                }
                let text = &src[byte_at(begin + 2)..byte_at(i)];
                let value = i64::from_str_radix(text, 16)
                    .map_err(|_| ParseError::new("Invalid hex number", start))?;
                tokens.push(Token {
                    tok: Tok::Int(value),
                    start,
                    end: byte_at(i),
                });
                continue;
            }
            while i < chars.len() {
                let ch = chars[i].1;
                if ch.is_ascii_digit() {
                    i += 1;
                } else if ch == '.' && !is_float {
                    is_float = true;
                    i += 1;
                } else if ch == 'e' || ch == 'E' {
                    is_float = true;
                    i += 1;
                    if matches!(chars.get(i).map(|&(_, c)| c), Some('+' | '-')) {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            let text = &src[start..byte_at(i)];
            let tok = if is_float {
                Tok::Double(
                    text.parse::<f64>()
                        .map_err(|_| ParseError::new("Invalid number", start))?,
                )
            } else {
                match text.parse::<i64>() {
                    Ok(v) => Tok::Int(v),
                    Err(_) => Tok::Double(
                        text.parse::<f64>()
                            .map_err(|_| ParseError::new("Invalid number", start))?,
                    ),
                }
            };
            tokens.push(Token {
                tok,
                start,
                end: byte_at(i),
            });
            continue;
        }

        // Identifiers (including `$match`-style operator keys)
        if is_ident_start(c) {
            let begin = i;
            while i < chars.len() && is_ident_char(chars[i].1) {
                i += 1;
            }
            tokens.push(Token {
                tok: Tok::Ident(src[byte_at(begin)..byte_at(i)].to_string()),
                start,
                end: byte_at(i),
            });
            continue;
        }

        // Regex literals
        if c == '/' && regex_allowed_after(tokens.last().map(|t| &t.tok)) {
            let mut pattern = String::new();
            let mut in_class = false;
            i += 1;
            loop {
                let Some(&(_, ch)) = chars.get(i) else {
                    return Err(ParseError::new("Unterminated regex", start));
                };
                if ch == '\n' {
                    return Err(ParseError::new("Unterminated regex", start));
                }
                i += 1;
                if ch == '\\' {
                    pattern.push(ch);
                    if let Some(&(_, esc)) = chars.get(i) {
                        pattern.push(esc);
                        i += 1;
                    }
                    continue;
                }
                if ch == '[' {
                    in_class = true;
                } else if ch == ']' {
                    in_class = false;
                } else if ch == '/' && !in_class {
                    break;
                }
                pattern.push(ch);
            }
            let flags_begin = i;
            while i < chars.len() && chars[i].1.is_ascii_alphabetic() {
                i += 1;
            }
            tokens.push(Token {
                tok: Tok::Regex {
                    pattern,
                    flags: src[byte_at(flags_begin)..byte_at(i)].to_string(),
                },
                start,
                end: byte_at(i),
            });
            continue;
        }

        if "(){}[],:.;-+".contains(c) {
            tokens.push(Token {
                tok: Tok::Punct(c),
                start,
                end: byte_at(i + 1),
            });
            i += 1;
            continue;
        }

        return Err(ParseError::new(
            format!("Unexpected character '{c}'"),
            start,
        ));
    }

    Ok(tokens)
}

// ============================================================================
// Parser
// ============================================================================

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
}

/// A parsed statement with its byte span in the source
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    pub statement: Statement,
    pub start: usize,
    pub end: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Result<Self, ParseError> {
        Ok(Self {
            src,
            tokens: tokenize(src)?,
            pos: 0,
        })
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|t| &t.tok)
    }

    fn offset(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map_or(self.src.len(), |t| t.start)
    }

    fn prev_end(&self) -> usize {
        self.pos
            .checked_sub(1)
            .and_then(|p| self.tokens.get(p))
            .map_or(0, |t| t.end)
    }

    fn bump(&mut self) -> Option<Tok> {
        let tok = self.tokens.get(self.pos).map(|t| t.tok.clone());
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn eat_punct(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::Punct(c)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, c: char) -> Result<(), ParseError> {
        if self.eat_punct(c) {
            Ok(())
        } else {
            Err(ParseError::new(
                format!("Expected '{c}'{}", self.found()),
                self.offset(),
            ))
        }
    }

    fn found(&self) -> String {
        match self.peek() {
            None => " but reached end of input".to_string(),
            Some(Tok::Ident(s)) => format!(" but found '{s}'"),
            Some(Tok::Str(_)) => " but found a string".to_string(),
            Some(Tok::Int(_) | Tok::Double(_)) => " but found a number".to_string(),
            Some(Tok::Regex { .. }) => " but found a regex".to_string(),
            Some(Tok::Punct(c)) => format!(" but found '{c}'"),
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<String, ParseError> {
        match self.peek() {
            Some(Tok::Ident(s)) => {
                let s = s.clone();
                self.pos += 1;
                Ok(s)
            }
            _ => Err(ParseError::new(
                format!("Expected {what}{}", self.found()),
                self.offset(),
            )),
        }
    }

    fn expect_string(&mut self, what: &str) -> Result<String, ParseError> {
        match self.peek() {
            Some(Tok::Str(s)) => {
                let s = s.clone();
                self.pos += 1;
                Ok(s)
            }
            _ => Err(ParseError::new(
                format!("Expected {what} as a string{}", self.found()),
                self.offset(),
            )),
        }
    }

    fn at_statement_start(&self) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if s == "db" || s == "show" || s == "use")
    }

    fn parse_all(&mut self) -> Result<Vec<Spanned>, ParseError> {
        let mut statements = Vec::new();
        loop {
            while self.eat_punct(';') {}
            if self.peek().is_none() {
                break;
            }
            let start = self.offset();
            let statement = self.parse_statement()?;
            let end = self.prev_end();
            statements.push(Spanned {
                statement,
                start,
                end,
            });
            // A statement ends at `;`, end of input, or where the next one begins
            if !self.eat_punct(';') && self.peek().is_some() && !self.at_statement_start() {
                return Err(ParseError::new(
                    format!("Expected ';' or a new statement{}", self.found()),
                    self.offset(),
                ));
            }
        }
        Ok(statements)
    }

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        let keyword = self.expect_ident("a statement starting with 'db', 'show' or 'use'")?;
        match keyword.as_str() {
            "show" => {
                let target = self.expect_ident("'dbs' or 'collections'")?;
                match target.as_str() {
                    "dbs" | "databases" => Ok(Statement::Show(ShowTarget::Databases)),
                    "collections" | "tables" => Ok(Statement::Show(ShowTarget::Collections)),
                    other => Err(ParseError::new(
                        format!(
                            "Unsupported 'show {other}' (use 'show dbs' or 'show collections')"
                        ),
                        self.prev_end(),
                    )),
                }
            }
            "use" => self.parse_use(),
            "db" => self.parse_db_expression(),
            other => Err(ParseError::new(
                format!("Statements must start with 'db', 'show' or 'use', found '{other}'"),
                self.tokens[self.pos - 1].start,
            )),
        }
    }

    /// `use <name>` — the name is the raw text up to end of line or `;`,
    /// so names like `my-db` work unquoted.
    fn parse_use(&mut self) -> Result<Statement, ParseError> {
        let start = self.prev_end();
        let rest = &self.src[start..];
        let line_end = rest.find(['\n', ';']).unwrap_or(rest.len());
        let name = rest[..line_end]
            .trim()
            .trim_matches(|c| c == '"' || c == '\'');
        if name.is_empty() {
            return Err(ParseError::new(
                "Expected a database name after 'use'",
                start,
            ));
        }
        let stop = start + line_end;
        while self.tokens.get(self.pos).is_some_and(|t| t.start < stop) {
            self.pos += 1;
        }
        Ok(Statement::Use(name.to_string()))
    }

    fn parse_db_expression(&mut self) -> Result<Statement, ParseError> {
        let mut database: Option<String> = None;
        let mut segments: Vec<String> = Vec::new();

        loop {
            if self.eat_punct('[') {
                if !segments.is_empty() {
                    return Err(ParseError::new(
                        "Unexpected '[' after a collection name",
                        self.offset(),
                    ));
                }
                segments.push(self.expect_string("a collection name")?);
                self.expect_punct(']')?;
                continue;
            }

            self.expect_punct('.')?;
            let name = self.expect_ident("a collection or method name")?;

            if self.peek() != Some(&Tok::Punct('(')) {
                segments.push(name);
                continue;
            }

            // `name(` — a method call
            if segments.is_empty() {
                match name.as_str() {
                    "getSiblingDB" => {
                        let args = self.parse_args()?;
                        database = Some(single_string_arg(&name, args, self.prev_end())?);
                        continue;
                    }
                    "getCollection" => {
                        let args = self.parse_args()?;
                        segments.push(single_string_arg(&name, args, self.prev_end())?);
                        continue;
                    }
                    _ => {
                        let args = self.parse_args()?;
                        return Ok(Statement::DatabaseCall {
                            database,
                            call: MethodCall { name, args },
                        });
                    }
                }
            }

            let args = self.parse_args()?;
            let call = MethodCall { name, args };
            let mut chain = Vec::new();
            while self.peek() == Some(&Tok::Punct('.')) {
                self.pos += 1;
                let name = self.expect_ident("a method name")?;
                let args = self.parse_args()?;
                chain.push(MethodCall { name, args });
            }
            return Ok(Statement::CollectionCall {
                database,
                collection: segments.join("."),
                call,
                chain,
            });
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Value>, ParseError> {
        self.expect_punct('(')?;
        let mut args = Vec::new();
        if self.eat_punct(')') {
            return Ok(args);
        }
        loop {
            args.push(self.parse_value()?);
            if self.eat_punct(',') {
                if self.eat_punct(')') {
                    break; // trailing comma
                }
                continue;
            }
            self.expect_punct(')')?;
            break;
        }
        Ok(args)
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        let offset = self.offset();
        match self.bump() {
            Some(Tok::Str(s)) => Ok(Value::Str(s)),
            Some(Tok::Int(v)) => Ok(Value::Int(v)),
            Some(Tok::Double(v)) => Ok(Value::Double(v)),
            Some(Tok::Regex { pattern, flags }) => Ok(Value::Regex { pattern, flags }),
            Some(Tok::Punct('-')) => match self.bump() {
                Some(Tok::Int(v)) => Ok(Value::Int(-v)),
                Some(Tok::Double(v)) => Ok(Value::Double(-v)),
                Some(Tok::Ident(s)) if s == "Infinity" => Ok(Value::Double(f64::NEG_INFINITY)),
                _ => Err(ParseError::new("Expected a number after '-'", offset)),
            },
            Some(Tok::Punct('+')) => match self.bump() {
                Some(Tok::Int(v)) => Ok(Value::Int(v)),
                Some(Tok::Double(v)) => Ok(Value::Double(v)),
                _ => Err(ParseError::new("Expected a number after '+'", offset)),
            },
            Some(Tok::Punct('{')) => self.parse_object(),
            Some(Tok::Punct('[')) => self.parse_array(),
            Some(Tok::Ident(ident)) => match ident.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                "null" => Ok(Value::Null),
                "undefined" => Ok(Value::Undefined),
                "Infinity" => Ok(Value::Double(f64::INFINITY)),
                "NaN" => Ok(Value::Double(f64::NAN)),
                "new" => {
                    let name = self.expect_ident("a constructor name after 'new'")?;
                    let args = self.parse_args()?;
                    Ok(Value::Call { name, args })
                }
                _ if self.peek() == Some(&Tok::Punct('(')) => {
                    let args = self.parse_args()?;
                    Ok(Value::Call { name: ident, args })
                }
                _ => Err(ParseError::new(
                    format!("Unknown identifier '{ident}' (variables are not supported)"),
                    offset,
                )),
            },
            Some(Tok::Punct(c)) => Err(ParseError::new(
                format!("Unexpected '{c}', expected a value"),
                offset,
            )),
            None => Err(ParseError::new(
                "Expected a value but reached end of input",
                offset,
            )),
        }
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        let mut fields: Vec<(String, Value)> = Vec::new();
        loop {
            if self.eat_punct('}') {
                break;
            }
            let offset = self.offset();
            let key = match self.bump() {
                Some(Tok::Ident(s)) | Some(Tok::Str(s)) => s,
                Some(Tok::Int(v)) => v.to_string(),
                Some(Tok::Double(v)) => v.to_string(),
                _ => return Err(ParseError::new("Expected a field name", offset)),
            };
            self.expect_punct(':')?;
            let value = self.parse_value()?;
            // Later duplicates win, as in JavaScript
            if let Some(existing) = fields.iter_mut().find(|(k, _)| *k == key) {
                existing.1 = value;
            } else {
                fields.push((key, value));
            }
            if !self.eat_punct(',') {
                self.expect_punct('}')?;
                break;
            }
        }
        Ok(Value::Object(fields))
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        let mut items = Vec::new();
        loop {
            if self.eat_punct(']') {
                break;
            }
            items.push(self.parse_value()?);
            if !self.eat_punct(',') {
                self.expect_punct(']')?;
                break;
            }
        }
        Ok(Value::Array(items))
    }
}

fn single_string_arg(method: &str, args: Vec<Value>, offset: usize) -> Result<String, ParseError> {
    match args.as_slice() {
        [Value::Str(s)] => Ok(s.clone()),
        _ => Err(ParseError::new(
            format!("{method}() expects a single string argument"),
            offset,
        )),
    }
}

/// Parse every statement in `src`, with byte spans.
pub fn parse_statements(src: &str) -> Result<Vec<Spanned>, ParseError> {
    Parser::new(src)?.parse_all()
}

/// Parse text that must contain exactly one statement.
pub fn parse_single(src: &str) -> Result<Statement, ParseError> {
    let mut statements = parse_statements(src)?;
    match statements.len() {
        1 => Ok(statements.remove(0).statement),
        0 => Err(ParseError::new("Nothing to execute", 0)),
        _ => Err(ParseError::new(
            "Expected a single statement",
            statements[1].start,
        )),
    }
}

/// Split editor text into statement source slices.
/// If the text does not parse, it is returned whole so execution reports the error.
pub fn split_statements(src: &str) -> Vec<String> {
    match parse_statements(src) {
        Ok(statements) => statements
            .iter()
            .map(|s| src[s.start..s.end].to_string())
            .collect(),
        Err(_) => {
            let trimmed = src.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![trimmed.to_string()]
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn obj(fields: Vec<(&str, Value)>) -> Value {
        Value::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    #[test]
    fn parses_find_with_chain() {
        let stmt = parse_single(
            "db.users.find({ age: { $gt: 30 } }, {name: 1}).sort({name: -1}).limit(10)",
        )
        .unwrap();
        let Statement::CollectionCall {
            database,
            collection,
            call,
            chain,
        } = stmt
        else {
            panic!("expected collection call");
        };
        assert_eq!(database, None);
        assert_eq!(collection, "users");
        assert_eq!(call.name, "find");
        assert_eq!(
            call.args,
            vec![
                obj(vec![("age", obj(vec![("$gt", Value::Int(30))]))]),
                obj(vec![("name", Value::Int(1))]),
            ]
        );
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].name, "sort");
        assert_eq!(chain[0].args, vec![obj(vec![("name", Value::Int(-1))])]);
        assert_eq!(chain[1].args, vec![Value::Int(10)]);
    }

    #[test]
    fn parses_collection_access_forms() {
        for src in [
            r#"db.getCollection("order-items").find()"#,
            r#"db["order-items"].find()"#,
        ] {
            match parse_single(src).unwrap() {
                Statement::CollectionCall { collection, .. } => {
                    assert_eq!(collection, "order-items")
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        match parse_single("db.system.profile.find()").unwrap() {
            Statement::CollectionCall { collection, .. } => {
                assert_eq!(collection, "system.profile")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_sibling_db_and_database_calls() {
        match parse_single(r#"db.getSiblingDB("admin").runCommand({ ping: 1 })"#).unwrap() {
            Statement::DatabaseCall { database, call } => {
                assert_eq!(database.as_deref(), Some("admin"));
                assert_eq!(call.name, "runCommand");
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse_single(r#"db.getSiblingDB('shop').orders.countDocuments({})"#).unwrap() {
            Statement::CollectionCall {
                database,
                collection,
                ..
            } => {
                assert_eq!(database.as_deref(), Some("shop"));
                assert_eq!(collection, "orders");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_shell_literals() {
        let stmt = parse_single(
            r#"db.c.find({
                _id: ObjectId("65a1f0c2e4b0a1b2c3d4e5f6"),
                at: { $gte: ISODate("2024-01-01"), $lt: new Date("2025-01-01T00:00:00Z") },
                name: /^jo/i,
                tags: ['a', "b",],
                'quoted key': -1.5e2,
                big: NumberLong("9007199254740993"),
                nothing: null, flag: true, // trailing comment
                /* block */ hex: 0xff,
            })"#,
        )
        .unwrap();
        let Statement::CollectionCall { call, .. } = stmt else {
            panic!()
        };
        let Value::Object(fields) = &call.args[0] else {
            panic!()
        };
        let get = |k: &str| {
            fields
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v)
                .unwrap()
        };
        assert_eq!(
            get("_id"),
            &Value::Call {
                name: "ObjectId".into(),
                args: vec![Value::Str("65a1f0c2e4b0a1b2c3d4e5f6".into())]
            }
        );
        assert_eq!(
            get("name"),
            &Value::Regex {
                pattern: "^jo".into(),
                flags: "i".into()
            }
        );
        assert_eq!(
            get("tags"),
            &Value::Array(vec![Value::Str("a".into()), Value::Str("b".into())])
        );
        assert_eq!(get("quoted key"), &Value::Double(-150.0));
        assert_eq!(get("nothing"), &Value::Null);
        assert_eq!(get("flag"), &Value::Bool(true));
        assert_eq!(get("hex"), &Value::Int(255));
        let Value::Object(at) = get("at") else {
            panic!()
        };
        assert!(matches!(&at[1].1, Value::Call { name, .. } if name == "Date"));
    }

    #[test]
    fn splits_multiple_statements_with_and_without_semicolons() {
        let src =
            "db.a.find()\ndb.b.find({x: 1});\n\nshow collections\nuse my-db\ndb.c.countDocuments()";
        let parts = split_statements(src);
        assert_eq!(
            parts,
            vec![
                "db.a.find()",
                "db.b.find({x: 1})",
                "show collections",
                "use my-db",
                "db.c.countDocuments()",
            ]
        );
        assert_eq!(
            parse_single("use my-db").unwrap(),
            Statement::Use("my-db".into())
        );
    }

    #[test]
    fn statement_split_ignores_separators_in_strings_and_comments() {
        let src = "db.a.find({ s: 'x; db.b.find()' }) // db.c.find()\ndb.d.find()";
        assert_eq!(
            split_statements(src),
            vec!["db.a.find({ s: 'x; db.b.find()' })", "db.d.find()"]
        );
    }

    #[test]
    fn invalid_text_is_returned_whole_for_error_reporting() {
        assert_eq!(
            split_statements("db.a.find({ x: })"),
            vec!["db.a.find({ x: })"]
        );
        assert!(split_statements("   ").is_empty());
    }

    #[test]
    fn rejects_javascript_outside_the_grammar() {
        assert!(parse_single("var x = 1").is_err());
        assert!(parse_single("db.a.find({ x: someVariable })").is_err());
        assert!(parse_single("db.a.find().forEach(printjson)").is_err());
    }

    #[test]
    fn error_messages_carry_line_and_column() {
        let src = "db.users.find({\n  age: \n})";
        let err = parse_single(src).unwrap_err();
        let msg = err.describe(src);
        assert!(msg.starts_with("Syntax error at line 3, column 1"), "{msg}");
    }

    #[test]
    fn show_statements() {
        assert_eq!(
            parse_single("show dbs").unwrap(),
            Statement::Show(ShowTarget::Databases)
        );
        assert_eq!(
            parse_single("show collections;").unwrap(),
            Statement::Show(ShowTarget::Collections)
        );
        assert!(parse_single("show users").is_err());
    }
}
