// SQL SELECT on MongoDB
//
// Parses a read-only SELECT subset and compiles it to an aggregation pipeline:
//
//   SELECT [DISTINCT] [TOP n] items
//   FROM collection [[AS] alias]
//   [[INNER | LEFT [OUTER]] JOIN collection [[AS] alias] ON a.x = b.y [AND ...]]...
//   [WHERE cond] [GROUP BY expr, ...] [HAVING cond]
//   [ORDER BY expr [ASC | DESC], ...]
//   [LIMIT n] [OFFSET n [ROWS]] [FETCH {FIRST | NEXT} n {ROW | ROWS} ONLY]
//
// Dotted names are nested fields (`address.city`) unless the first part is a
// FROM/JOIN alias. Identifiers may be quoted as "x", [x] or `x`. Shell helpers
// work as literals: ObjectId('...'), ISODate('...'), also DATE '2024-01-01'.
// Anything else (subqueries, UNION, RIGHT/FULL JOIN, writes) is rejected with
// a message pointing at the unsupported part.

use mongodb::bson::{doc, Bson, Document, Regex};

use super::convert;
use super::shell::Value as ShellValue;

pub type SqlResult<T> = Result<T, String>;

// ============================================================================
// Lexer
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// Identifier or keyword; `quoted` identifiers are never keywords
    Ident {
        text: String,
        quoted: bool,
    },
    Int(i64),
    Float(f64),
    Str(String),
    Op(&'static str),
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    offset: usize,
}

const OPERATORS: &[&str] = &[
    "<>", "!=", "<=", ">=", "=", "<", ">", "+", "-", "*", "/", "%", "(", ")", ",", ".", ";",
];

fn tokenize(src: &str) -> SqlResult<Vec<Token>> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < src.len() {
        let c = src[i..].chars().next().unwrap_or(' ');
        let rest = &src[i..];
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        if rest.starts_with("--") || rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            i += rest.find("*/").map_or(rest.len(), |p| p + 2);
            continue;
        }
        let offset = i;
        // N'unicode' strings are plain strings here
        if (c == 'N' || c == 'n') && bytes.get(i + 1) == Some(&b'\'') {
            i += 1;
            continue;
        }
        if c == '\'' {
            let mut value = String::new();
            i += 1;
            loop {
                let Some(ch) = src[i..].chars().next() else {
                    return Err(error_at(src, offset, "Unterminated string"));
                };
                i += ch.len_utf8();
                if ch == '\'' {
                    if src[i..].starts_with('\'') {
                        value.push('\'');
                        i += 1;
                        continue;
                    }
                    break;
                }
                value.push(ch);
            }
            tokens.push(Token {
                tok: Tok::Str(value),
                offset,
            });
            continue;
        }
        if c == '"' || c == '`' || c == '[' {
            let close = if c == '[' { ']' } else { c };
            let body_start = i + 1;
            let Some(len) = src[body_start..].find(close) else {
                return Err(error_at(src, offset, "Unterminated quoted identifier"));
            };
            tokens.push(Token {
                tok: Tok::Ident {
                    text: src[body_start..body_start + len].to_string(),
                    quoted: true,
                },
                offset,
            });
            i = body_start + len + 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && rest[1..].starts_with(|d: char| d.is_ascii_digit())) {
            let mut end = i;
            let mut is_float = false;
            while end < src.len() {
                let d = bytes[end];
                if d.is_ascii_digit() {
                    end += 1;
                } else if d == b'.' && !is_float {
                    is_float = true;
                    end += 1;
                } else if (d == b'e' || d == b'E') && end > i {
                    is_float = true;
                    end += 1;
                    if matches!(bytes.get(end), Some(b'+' | b'-')) {
                        end += 1;
                    }
                } else {
                    break;
                }
            }
            let text = &src[i..end];
            let tok = if is_float {
                Tok::Float(
                    text.parse()
                        .map_err(|_| error_at(src, offset, "Invalid number"))?,
                )
            } else {
                match text.parse::<i64>() {
                    Ok(v) => Tok::Int(v),
                    Err(_) => Tok::Float(
                        text.parse()
                            .map_err(|_| error_at(src, offset, "Invalid number"))?,
                    ),
                }
            };
            tokens.push(Token { tok, offset });
            i = end;
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' || c == '@' {
            let end = rest
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '@'))
                .map_or(src.len(), |p| i + p);
            tokens.push(Token {
                tok: Tok::Ident {
                    text: src[i..end].to_string(),
                    quoted: false,
                },
                offset,
            });
            i = end;
            continue;
        }
        if let Some(op) = OPERATORS.iter().find(|op| rest.starts_with(**op)) {
            tokens.push(Token {
                tok: Tok::Op(op),
                offset,
            });
            i += op.len();
            continue;
        }
        return Err(error_at(
            src,
            offset,
            &format!("Unexpected character '{c}'"),
        ));
    }
    Ok(tokens)
}

fn error_at(src: &str, offset: usize, message: &str) -> String {
    let upto = &src[..offset.min(src.len())];
    let line = upto.matches('\n').count() + 1;
    let column = upto.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    format!("SQL error at line {line}, column {column}: {message}")
}

// ============================================================================
// AST
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// DATE '...' / TIMESTAMP '...'
    Date(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(Vec<String>),
    Literal(Lit),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    IsNull {
        expr: Box<Expr>,
        negated: bool,
    },
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negated: bool,
    },
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        negated: bool,
    },
    Like {
        expr: Box<Expr>,
        pattern: Box<Expr>,
        negated: bool,
        case_insensitive: bool,
    },
    Function {
        name: String,
        args: Vec<Expr>,
        distinct: bool,
        star: bool,
    },
    Case {
        operand: Option<Box<Expr>>,
        whens: Vec<(Expr, Expr)>,
        otherwise: Option<Box<Expr>>,
    },
    Cast {
        expr: Box<Expr>,
        ty: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectItem {
    /// `*` or `alias.*`
    Wildcard,
    Expr {
        expr: Expr,
        alias: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    pub left: bool,
    pub table: TableRef,
    pub on: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub distinct: bool,
    pub items: Vec<SelectItem>,
    pub from: TableRef,
    pub joins: Vec<Join>,
    pub filter: Option<Expr>,
    pub group_by: Vec<Expr>,
    pub having: Option<Expr>,
    pub order_by: Vec<(Expr, bool)>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

// ============================================================================
// Parser
// ============================================================================

/// Words that end an expression or alias position
const RESERVED: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP",
    "BY",
    "HAVING",
    "ORDER",
    "LIMIT",
    "OFFSET",
    "FETCH",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "FULL",
    "OUTER",
    "CROSS",
    "ON",
    "AND",
    "OR",
    "NOT",
    "AS",
    "UNION",
    "EXCEPT",
    "INTERSECT",
    "ASC",
    "DESC",
    "IS",
    "IN",
    "LIKE",
    "ILIKE",
    "BETWEEN",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "DISTINCT",
    "TOP",
    "INTO",
    "ROWS",
    "ROW",
    "ONLY",
    "NULL",
    "TRUE",
    "FALSE",
];

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn offset(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map_or(self.src.len(), |t| t.offset)
    }

    fn err<T>(&self, message: &str) -> SqlResult<T> {
        Err(error_at(self.src, self.offset(), message))
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|t| &t.tok)
    }

    fn peek_keyword(&self, kw: &str) -> bool {
        self.keyword_at(self.pos, kw)
    }

    fn keyword_at(&self, pos: usize, kw: &str) -> bool {
        matches!(self.tokens.get(pos).map(|t| &t.tok), Some(Tok::Ident { text, quoted: false }) if text.eq_ignore_ascii_case(kw))
    }

    fn eat_keyword(&mut self, kw: &str) -> bool {
        if self.peek_keyword(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_keyword(&mut self, kw: &str) -> SqlResult<()> {
        if self.eat_keyword(kw) {
            Ok(())
        } else {
            self.err(&format!("Expected {kw}{}", self.found()))
        }
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Op(o)) if *o == op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_op(&mut self, op: &str) -> SqlResult<()> {
        if self.eat_op(op) {
            Ok(())
        } else {
            self.err(&format!("Expected '{op}'{}", self.found()))
        }
    }

    fn found(&self) -> String {
        match self.peek() {
            None => " but reached the end of the query".to_string(),
            Some(Tok::Ident { text, .. }) => format!(" but found '{text}'"),
            Some(Tok::Op(op)) => format!(" but found '{op}'"),
            Some(Tok::Str(_)) => " but found a string".to_string(),
            Some(Tok::Int(_) | Tok::Float(_)) => " but found a number".to_string(),
        }
    }

    fn is_reserved_here(&self) -> bool {
        matches!(self.peek(), Some(Tok::Ident { text, quoted: false }) if RESERVED.iter().any(|r| text.eq_ignore_ascii_case(r)))
    }

    /// An identifier that is not a reserved word
    fn ident(&mut self, what: &str) -> SqlResult<String> {
        match self.peek() {
            Some(Tok::Ident { text, quoted }) if *quoted || !self.is_reserved_here() => {
                let text = text.clone();
                self.pos += 1;
                Ok(text)
            }
            _ => self.err(&format!("Expected {what}{}", self.found())),
        }
    }

    fn optional_alias(&mut self) -> SqlResult<Option<String>> {
        if self.eat_keyword("AS") {
            return match self.peek() {
                Some(Tok::Str(s)) => {
                    let s = s.clone();
                    self.pos += 1;
                    Ok(Some(s))
                }
                _ => self.ident("an alias").map(Some),
            };
        }
        match self.peek() {
            Some(Tok::Ident { quoted, .. }) if *quoted || !self.is_reserved_here() => {
                self.ident("an alias").map(Some)
            }
            _ => Ok(None),
        }
    }

    fn count(&mut self, what: &str) -> SqlResult<u64> {
        let parens = self.eat_op("(");
        let value = match self.peek() {
            Some(Tok::Int(n)) if *n >= 0 => *n as u64,
            _ => return self.err(&format!("Expected a non-negative integer for {what}")),
        };
        self.pos += 1;
        if parens {
            self.expect_op(")")?;
        }
        Ok(value)
    }

    fn parse_query(&mut self) -> SqlResult<Query> {
        if !self.eat_keyword("SELECT") {
            return self.err("SQL on MongoDB supports SELECT only — use mongo-shell syntax (db.collection.insertOne(...)) for writes");
        }
        let distinct = self.eat_keyword("DISTINCT");
        let mut limit = None;
        if self.eat_keyword("TOP") {
            limit = Some(self.count("TOP")?);
        }

        let mut items = Vec::new();
        loop {
            items.push(self.parse_select_item()?);
            if !self.eat_op(",") {
                break;
            }
        }

        if self.peek_keyword("INTO") {
            return self.err("SELECT ... INTO is not supported");
        }
        self.expect_keyword("FROM")?;
        let from = self.parse_table_ref()?;
        if self.eat_op(",") {
            return self.err("Comma joins are not supported — use JOIN ... ON");
        }

        let mut joins = Vec::new();
        loop {
            let left = if self.eat_keyword("LEFT") {
                self.eat_keyword("OUTER");
                true
            } else if self.eat_keyword("INNER") {
                false
            } else if self.peek_keyword("RIGHT")
                || self.peek_keyword("FULL")
                || self.peek_keyword("CROSS")
            {
                return self.err("Only INNER and LEFT joins are supported");
            } else if self.peek_keyword("JOIN") {
                false
            } else {
                break;
            };
            self.expect_keyword("JOIN")?;
            let table = self.parse_table_ref()?;
            self.expect_keyword("ON")?;
            let on = self.parse_expr()?;
            joins.push(Join { left, table, on });
        }

        let filter = if self.eat_keyword("WHERE") {
            Some(self.parse_expr()?)
        } else {
            None
        };

        let mut group_by = Vec::new();
        if self.eat_keyword("GROUP") {
            self.expect_keyword("BY")?;
            loop {
                group_by.push(self.parse_expr()?);
                if !self.eat_op(",") {
                    break;
                }
            }
        }
        let having = if self.eat_keyword("HAVING") {
            Some(self.parse_expr()?)
        } else {
            None
        };

        let mut order_by = Vec::new();
        if self.eat_keyword("ORDER") {
            self.expect_keyword("BY")?;
            loop {
                let expr = self.parse_expr()?;
                let desc = if self.eat_keyword("DESC") {
                    true
                } else {
                    self.eat_keyword("ASC");
                    false
                };
                order_by.push((expr, desc));
                if !self.eat_op(",") {
                    break;
                }
            }
        }

        let mut offset = None;
        loop {
            if self.eat_keyword("LIMIT") {
                if limit.is_some() {
                    return self.err("Use either TOP or LIMIT, not both");
                }
                limit = Some(self.count("LIMIT")?);
            } else if self.eat_keyword("OFFSET") {
                offset = Some(self.count("OFFSET")?);
                if !self.eat_keyword("ROWS") {
                    self.eat_keyword("ROW");
                }
            } else if self.eat_keyword("FETCH") {
                if !self.eat_keyword("FIRST") {
                    self.expect_keyword("NEXT")?;
                }
                if limit.is_some() {
                    return self.err("Use only one of TOP, LIMIT or FETCH");
                }
                limit = Some(self.count("FETCH")?);
                if !self.eat_keyword("ROWS") {
                    self.expect_keyword("ROW")?;
                }
                self.expect_keyword("ONLY")?;
            } else {
                break;
            }
        }

        self.eat_op(";");
        if self.peek_keyword("UNION")
            || self.peek_keyword("EXCEPT")
            || self.peek_keyword("INTERSECT")
        {
            return self.err("UNION / EXCEPT / INTERSECT are not supported");
        }
        if self.peek().is_some() {
            return self.err(&format!("Unexpected text after the query{}", self.found()));
        }

        Ok(Query {
            distinct,
            items,
            from,
            joins,
            filter,
            group_by,
            having,
            order_by,
            limit,
            offset,
        })
    }

    fn parse_select_item(&mut self) -> SqlResult<SelectItem> {
        if self.eat_op("*") {
            return Ok(SelectItem::Wildcard);
        }
        // alias.*
        if matches!(self.peek(), Some(Tok::Ident { .. }))
            && matches!(
                self.tokens.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::Op("."))
            )
            && matches!(
                self.tokens.get(self.pos + 2).map(|t| &t.tok),
                Some(Tok::Op("*"))
            )
        {
            self.pos += 3;
            return Ok(SelectItem::Wildcard);
        }
        let expr = self.parse_expr()?;
        let alias = self.optional_alias()?;
        Ok(SelectItem::Expr { expr, alias })
    }

    fn parse_table_ref(&mut self) -> SqlResult<TableRef> {
        if self.peek() == Some(&Tok::Op("(")) {
            return self.err("Subqueries are not supported");
        }
        let mut parts = vec![self.ident("a collection name")?];
        while self.eat_op(".") {
            parts.push(self.ident("a collection name")?);
        }
        let alias = self.optional_alias()?;
        Ok(TableRef {
            name: parts.join("."),
            alias,
        })
    }

    fn parse_expr(&mut self) -> SqlResult<Expr> {
        let mut left = self.parse_and()?;
        while self.eat_keyword("OR") {
            let right = self.parse_and()?;
            left = Expr::Binary(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> SqlResult<Expr> {
        let mut left = self.parse_not()?;
        while self.eat_keyword("AND") {
            let right = self.parse_not()?;
            left = Expr::Binary(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> SqlResult<Expr> {
        if self.eat_keyword("NOT") {
            return Ok(Expr::Not(Box::new(self.parse_not()?)));
        }
        self.parse_predicate()
    }

    fn parse_predicate(&mut self) -> SqlResult<Expr> {
        let left = self.parse_additive()?;

        if self.eat_keyword("IS") {
            let negated = self.eat_keyword("NOT");
            self.expect_keyword("NULL")?;
            return Ok(Expr::IsNull {
                expr: Box::new(left),
                negated,
            });
        }

        let negated = if self.peek_keyword("NOT")
            && (self.keyword_at(self.pos + 1, "IN")
                || self.keyword_at(self.pos + 1, "LIKE")
                || self.keyword_at(self.pos + 1, "ILIKE")
                || self.keyword_at(self.pos + 1, "BETWEEN"))
        {
            self.pos += 1;
            true
        } else {
            false
        };

        if self.eat_keyword("IN") {
            self.expect_op("(")?;
            if self.peek_keyword("SELECT") {
                return self.err("Subqueries are not supported");
            }
            let mut list = Vec::new();
            loop {
                list.push(self.parse_additive()?);
                if !self.eat_op(",") {
                    break;
                }
            }
            self.expect_op(")")?;
            return Ok(Expr::InList {
                expr: Box::new(left),
                list,
                negated,
            });
        }
        let case_insensitive = self.peek_keyword("ILIKE");
        if self.eat_keyword("LIKE") || self.eat_keyword("ILIKE") {
            let pattern = self.parse_additive()?;
            return Ok(Expr::Like {
                expr: Box::new(left),
                pattern: Box::new(pattern),
                negated,
                case_insensitive,
            });
        }
        if self.eat_keyword("BETWEEN") {
            let low = self.parse_additive()?;
            self.expect_keyword("AND")?;
            let high = self.parse_additive()?;
            return Ok(Expr::Between {
                expr: Box::new(left),
                low: Box::new(low),
                high: Box::new(high),
                negated,
            });
        }
        if negated {
            return self.err("Expected IN, LIKE or BETWEEN after NOT");
        }

        let op = match self.peek() {
            Some(Tok::Op("=")) => Some(BinOp::Eq),
            Some(Tok::Op("<>" | "!=")) => Some(BinOp::Ne),
            Some(Tok::Op("<")) => Some(BinOp::Lt),
            Some(Tok::Op("<=")) => Some(BinOp::Le),
            Some(Tok::Op(">")) => Some(BinOp::Gt),
            Some(Tok::Op(">=")) => Some(BinOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.pos += 1;
            let right = self.parse_additive()?;
            return Ok(Expr::Binary(op, Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> SqlResult<Expr> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = if self.eat_op("+") {
                BinOp::Add
            } else if self.eat_op("-") {
                BinOp::Sub
            } else {
                break;
            };
            let right = self.parse_multiplicative()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> SqlResult<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = if self.eat_op("*") {
                BinOp::Mul
            } else if self.eat_op("/") {
                BinOp::Div
            } else if self.eat_op("%") {
                BinOp::Mod
            } else {
                break;
            };
            let right = self.parse_unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> SqlResult<Expr> {
        if self.eat_op("-") {
            return Ok(match self.parse_unary()? {
                Expr::Literal(Lit::Int(v)) => Expr::Literal(Lit::Int(-v)),
                Expr::Literal(Lit::Float(v)) => Expr::Literal(Lit::Float(-v)),
                other => Expr::Neg(Box::new(other)),
            });
        }
        if self.eat_op("+") {
            return self.parse_unary();
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> SqlResult<Expr> {
        let Some(tok) = self.peek().cloned() else {
            return self.err("Expected an expression but reached the end of the query");
        };
        match tok {
            Tok::Int(v) => {
                self.pos += 1;
                Ok(Expr::Literal(Lit::Int(v)))
            }
            Tok::Float(v) => {
                self.pos += 1;
                Ok(Expr::Literal(Lit::Float(v)))
            }
            Tok::Str(s) => {
                self.pos += 1;
                Ok(Expr::Literal(Lit::Str(s)))
            }
            Tok::Op("(") => {
                self.pos += 1;
                if self.peek_keyword("SELECT") {
                    return self.err("Subqueries are not supported");
                }
                let expr = self.parse_expr()?;
                self.expect_op(")")?;
                Ok(expr)
            }
            Tok::Op(op) => self.err(&format!("Unexpected '{op}', expected an expression")),
            Tok::Ident { text, quoted } => {
                if !quoted {
                    let upper = text.to_ascii_uppercase();
                    match upper.as_str() {
                        "NULL" => {
                            self.pos += 1;
                            return Ok(Expr::Literal(Lit::Null));
                        }
                        "TRUE" | "FALSE" => {
                            self.pos += 1;
                            return Ok(Expr::Literal(Lit::Bool(upper == "TRUE")));
                        }
                        "DATE" | "TIMESTAMP" | "DATETIME"
                            if matches!(
                                self.tokens.get(self.pos + 1).map(|t| &t.tok),
                                Some(Tok::Str(_))
                            ) =>
                        {
                            self.pos += 1;
                            let Some(Tok::Str(s)) = self.peek().cloned() else {
                                return self.err("Expected a date string");
                            };
                            self.pos += 1;
                            return Ok(Expr::Literal(Lit::Date(s)));
                        }
                        "CASE" => {
                            self.pos += 1;
                            return self.parse_case();
                        }
                        "CAST" => {
                            self.pos += 1;
                            self.expect_op("(")?;
                            let expr = self.parse_expr()?;
                            self.expect_keyword("AS")?;
                            let ty = self.ident("a type name")?;
                            // Ignore precision, e.g. NVARCHAR(50) or DECIMAL(10, 2)
                            if self.eat_op("(") {
                                while !self.eat_op(")") {
                                    if self.peek().is_none() {
                                        return self.err("Expected ')'");
                                    }
                                    self.pos += 1;
                                }
                            }
                            self.expect_op(")")?;
                            return Ok(Expr::Cast {
                                expr: Box::new(expr),
                                ty: ty.to_ascii_uppercase(),
                            });
                        }
                        "CURRENT_TIMESTAMP" => {
                            self.pos += 1;
                            return Ok(Expr::Function {
                                name: "NOW".into(),
                                args: vec![],
                                distinct: false,
                                star: false,
                            });
                        }
                        "SELECT" => return self.err("Subqueries are not supported"),
                        _ => {}
                    }
                    if self.is_reserved_here() {
                        return self.err(&format!("Unexpected '{text}', expected an expression"));
                    }
                }
                self.pos += 1;

                // Function call
                if !quoted && self.peek() == Some(&Tok::Op("(")) {
                    self.pos += 1;
                    let distinct = self.eat_keyword("DISTINCT");
                    let mut args = Vec::new();
                    let mut star = false;
                    if self.eat_op("*") {
                        star = true;
                    } else if self.peek() != Some(&Tok::Op(")")) {
                        loop {
                            args.push(self.parse_expr()?);
                            if !self.eat_op(",") {
                                break;
                            }
                        }
                    }
                    self.expect_op(")")?;
                    if self.peek_keyword("OVER") {
                        return self.err("Window functions (OVER) are not supported");
                    }
                    return Ok(Expr::Function {
                        name: text,
                        args,
                        distinct,
                        star,
                    });
                }

                // Column, possibly dotted
                let mut parts = vec![text];
                while self.peek() == Some(&Tok::Op(".")) {
                    self.pos += 1;
                    parts.push(self.ident("a field name")?);
                }
                Ok(Expr::Column(parts))
            }
        }
    }

    fn parse_case(&mut self) -> SqlResult<Expr> {
        let operand = if self.peek_keyword("WHEN") {
            None
        } else {
            Some(Box::new(self.parse_expr()?))
        };
        let mut whens = Vec::new();
        while self.eat_keyword("WHEN") {
            let cond = self.parse_expr()?;
            self.expect_keyword("THEN")?;
            let value = self.parse_expr()?;
            whens.push((cond, value));
        }
        if whens.is_empty() {
            return self.err("CASE needs at least one WHEN");
        }
        let otherwise = if self.eat_keyword("ELSE") {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        self.expect_keyword("END")?;
        Ok(Expr::Case {
            operand,
            whens,
            otherwise,
        })
    }
}

pub fn parse(src: &str) -> SqlResult<Query> {
    let mut parser = Parser {
        src,
        tokens: tokenize(src)?,
        pos: 0,
    };
    parser.parse_query()
}

// ============================================================================
// Compiler
// ============================================================================

/// A query compiled to an aggregation pipeline
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    pub collection: String,
    pub pipeline: Vec<Document>,
    /// Output columns in SELECT order (`None` for SELECT *). $project omits
    /// missing fields, so the grid uses this list to show them as NULL.
    pub columns: Option<Vec<String>>,
}

struct JoinScope {
    names: Vec<String>,
    field: String,
}

/// Resolves column references to document paths
struct Scope {
    base: Vec<String>,
    joins: Vec<JoinScope>,
}

impl Scope {
    fn new(from: &TableRef) -> Self {
        let mut base = vec![from.name.clone()];
        base.extend(from.alias.clone());
        Self {
            base,
            joins: Vec::new(),
        }
    }

    fn matches(names: &[String], part: &str) -> bool {
        names.iter().any(|n| n.eq_ignore_ascii_case(part))
    }

    /// Path for a column; the second value is the join it belongs to, if any
    fn resolve(&self, parts: &[String]) -> (String, Option<usize>) {
        if parts.len() > 1 {
            if Self::matches(&self.base, &parts[0]) {
                return (parts[1..].join("."), None);
            }
            if let Some(idx) = self
                .joins
                .iter()
                .position(|j| Self::matches(&j.names, &parts[0]))
            {
                return (
                    format!("{}.{}", self.joins[idx].field, parts[1..].join(".")),
                    Some(idx),
                );
            }
        }
        (parts.join("."), None)
    }

    fn path(&self, parts: &[String]) -> String {
        self.resolve(parts).0
    }
}

const SHELL_HELPERS: &[&str] = &[
    "ObjectId",
    "ISODate",
    "NumberInt",
    "NumberLong",
    "NumberDecimal",
    "UUID",
    "Timestamp",
    "MinKey",
    "MaxKey",
];

fn literal(lit: &Lit) -> SqlResult<Bson> {
    Ok(match lit {
        Lit::Null => Bson::Null,
        Lit::Bool(b) => Bson::Boolean(*b),
        Lit::Int(v) => match i32::try_from(*v) {
            Ok(small) => Bson::Int32(small),
            Err(_) => Bson::Int64(*v),
        },
        Lit::Float(v) => Bson::Double(*v),
        Lit::Str(s) => Bson::String(s.clone()),
        Lit::Date(s) => convert::to_bson(&ShellValue::Call {
            name: "ISODate".into(),
            args: vec![ShellValue::Str(s.clone())],
        })?,
    })
}

/// A constant value, if `expr` is one (literals and shell helpers like ObjectId('...'))
fn constant(expr: &Expr) -> SqlResult<Option<Bson>> {
    match expr {
        Expr::Literal(lit) => literal(lit).map(Some),
        Expr::Function {
            name,
            args,
            distinct: false,
            star: false,
        } => {
            let Some(helper) = SHELL_HELPERS.iter().find(|h| h.eq_ignore_ascii_case(name)) else {
                return Ok(None);
            };
            let mut shell_args = Vec::new();
            for arg in args {
                shell_args.push(match arg {
                    Expr::Literal(Lit::Str(s)) => ShellValue::Str(s.clone()),
                    Expr::Literal(Lit::Int(v)) => ShellValue::Int(*v),
                    Expr::Literal(Lit::Float(v)) => ShellValue::Double(*v),
                    _ => return Err(format!("{helper}() expects literal arguments")),
                });
            }
            convert::to_bson(&ShellValue::Call {
                name: (*helper).to_string(),
                args: shell_args,
            })
            .map(Some)
        }
        _ => Ok(None),
    }
}

/// Aggregation-expression form of a constant (strings starting with `$` need $literal)
fn literal_expr(value: Bson) -> Bson {
    match &value {
        Bson::String(s) if s.starts_with('$') => Bson::Document(doc! { "$literal": value }),
        Bson::Array(_) | Bson::Document(_) => Bson::Document(doc! { "$literal": value }),
        _ => value,
    }
}

fn like_regex(pattern: &str, case_insensitive: bool) -> Regex {
    let mut re = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' => re.push_str(".*"),
            '_' => re.push('.'),
            // [abc] character classes, as in SQL Server
            '[' => {
                let mut class = String::new();
                let mut closed = false;
                for d in chars.by_ref() {
                    if d == ']' {
                        closed = true;
                        break;
                    }
                    class.push(d);
                }
                if closed && !class.is_empty() {
                    let body = class
                        .strip_prefix('^')
                        .map(|rest| format!("^{}", escape_class(rest)))
                        .unwrap_or_else(|| escape_class(&class));
                    re.push('[');
                    re.push_str(&body);
                    re.push(']');
                } else {
                    re.push_str("\\[");
                    re.push_str(&regex_escape(&class));
                }
            }
            other => re.push_str(&regex_escape(&other.to_string())),
        }
    }
    re.push('$');
    // `s` so `%` also matches newlines, like SQL
    Regex {
        pattern: re,
        options: if case_insensitive {
            "is".into()
        } else {
            "s".into()
        },
    }
}

fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "\\^$.|?*+()[]{}/".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn escape_class(s: &str) -> String {
    s.chars()
        .map(|c| {
            if matches!(c, '\\' | ']' | '/') {
                format!("\\{c}")
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn flip(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Gt,
        BinOp::Le => BinOp::Ge,
        BinOp::Gt => BinOp::Lt,
        BinOp::Ge => BinOp::Le,
        other => other,
    }
}

fn query_op(op: BinOp) -> Option<&'static str> {
    Some(match op {
        BinOp::Eq => "$eq",
        BinOp::Ne => "$ne",
        BinOp::Lt => "$lt",
        BinOp::Le => "$lte",
        BinOp::Gt => "$gt",
        BinOp::Ge => "$gte",
        _ => return None,
    })
}

fn is_aggregate_name(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "COUNT"
            | "SUM"
            | "AVG"
            | "MIN"
            | "MAX"
            | "STDEV"
            | "STDEVP"
            | "STDDEV"
            | "STDDEV_POP"
            | "STDDEV_SAMP"
    )
}

fn contains_aggregate(expr: &Expr) -> bool {
    let mut found = false;
    walk(expr, &mut |e| {
        if let Expr::Function { name, .. } = e {
            if is_aggregate_name(name) {
                found = true;
            }
        }
    });
    found
}

fn walk(expr: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(expr);
    match expr {
        Expr::Column(_) | Expr::Literal(_) => {}
        Expr::Neg(e) | Expr::Not(e) | Expr::Cast { expr: e, .. } => walk(e, f),
        Expr::Binary(_, l, r) => {
            walk(l, f);
            walk(r, f);
        }
        Expr::IsNull { expr, .. } => walk(expr, f),
        Expr::InList { expr, list, .. } => {
            walk(expr, f);
            list.iter().for_each(|e| walk(e, f));
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            walk(expr, f);
            walk(low, f);
            walk(high, f);
        }
        Expr::Like { expr, pattern, .. } => {
            walk(expr, f);
            walk(pattern, f);
        }
        Expr::Function { args, .. } => args.iter().for_each(|e| walk(e, f)),
        Expr::Case {
            operand,
            whens,
            otherwise,
        } => {
            if let Some(o) = operand {
                walk(o, f);
            }
            for (c, v) in whens {
                walk(c, f);
                walk(v, f);
            }
            if let Some(o) = otherwise {
                walk(o, f);
            }
        }
    }
}

/// How aggregates and group keys are referenced after a $group stage
struct GroupContext<'a> {
    keys: &'a [(Expr, String)],
    aggregates: &'a [(Expr, String, AggKind)],
}

#[derive(Clone, Copy, PartialEq)]
enum AggKind {
    Plain,
    /// $addToSet; the reference counts distinct non-null values
    CountDistinct,
}

struct Compiler<'a> {
    scope: Scope,
    group: Option<GroupContext<'a>>,
}

impl Compiler<'_> {
    fn same(&self, a: &Expr, b: &Expr) -> bool {
        match (a, b) {
            (Expr::Column(x), Expr::Column(y)) => self.scope.path(x) == self.scope.path(y),
            _ => a == b,
        }
    }

    /// Aggregation expression for `expr`
    fn expr(&self, expr: &Expr) -> SqlResult<Bson> {
        if let Some(group) = &self.group {
            if let Some((_, name)) = group.keys.iter().find(|(k, _)| self.same(k, expr)) {
                return Ok(Bson::String(format!("$_id.{name}")));
            }
            if let Some((_, name, kind)) = group.aggregates.iter().find(|(a, _, _)| a == expr) {
                return Ok(match kind {
                    AggKind::Plain => Bson::String(format!("${name}")),
                    AggKind::CountDistinct => Bson::Document(doc! {
                        "$size": { "$filter": { "input": format!("${name}"), "cond": { "$gt": ["$$this", Bson::Null] } } }
                    }),
                });
            }
            if let Expr::Column(parts) = expr {
                return Err(format!(
                    "Column '{}' must appear in GROUP BY or inside an aggregate such as COUNT() or SUM()",
                    parts.join(".")
                ));
            }
        }

        if let Some(value) = constant(expr)? {
            return Ok(literal_expr(value));
        }

        Ok(match expr {
            Expr::Column(parts) => Bson::String(format!("${}", self.scope.path(parts))),
            Expr::Literal(_) => unreachable_literal()?,
            Expr::Neg(e) => Bson::Document(doc! { "$multiply": [self.expr(e)?, -1] }),
            Expr::Not(e) => Bson::Document(doc! { "$not": [self.expr(e)?] }),
            Expr::Binary(op, l, r) => {
                let name = match op {
                    BinOp::And => "$and",
                    BinOp::Or => "$or",
                    BinOp::Eq => "$eq",
                    BinOp::Ne => "$ne",
                    BinOp::Lt => "$lt",
                    BinOp::Le => "$lte",
                    BinOp::Gt => "$gt",
                    BinOp::Ge => "$gte",
                    BinOp::Add => "$add",
                    BinOp::Sub => "$subtract",
                    BinOp::Mul => "$multiply",
                    BinOp::Div => "$divide",
                    BinOp::Mod => "$mod",
                };
                Bson::Document(doc! { name: [self.expr(l)?, self.expr(r)?] })
            }
            // `$lte: [x, null]` is true for null *and* missing fields
            Expr::IsNull { expr, negated } => {
                let op = if *negated { "$gt" } else { "$lte" };
                Bson::Document(doc! { op: [self.expr(expr)?, Bson::Null] })
            }
            Expr::InList {
                expr,
                list,
                negated,
            } => {
                let values = list
                    .iter()
                    .map(|e| self.expr(e))
                    .collect::<SqlResult<Vec<_>>>()?;
                let test = doc! { "$in": [self.expr(expr)?, Bson::Array(values)] };
                if *negated {
                    Bson::Document(doc! { "$not": [test] })
                } else {
                    Bson::Document(test)
                }
            }
            Expr::Between {
                expr,
                low,
                high,
                negated,
            } => {
                let value = self.expr(expr)?;
                let test = doc! { "$and": [
                    { "$gte": [value.clone(), self.expr(low)?] },
                    { "$lte": [value, self.expr(high)?] },
                ] };
                if *negated {
                    Bson::Document(doc! { "$not": [test] })
                } else {
                    Bson::Document(test)
                }
            }
            Expr::Like {
                expr,
                pattern,
                negated,
                case_insensitive,
            } => {
                let Expr::Literal(Lit::Str(pattern)) = pattern.as_ref() else {
                    return Err("LIKE needs a string pattern".into());
                };
                let re = like_regex(pattern, *case_insensitive);
                let test = doc! { "$regexMatch": { "input": self.expr(expr)?, "regex": re.pattern, "options": re.options } };
                if *negated {
                    Bson::Document(doc! { "$not": [test] })
                } else {
                    Bson::Document(test)
                }
            }
            Expr::Case {
                operand,
                whens,
                otherwise,
            } => {
                let mut branches = Vec::new();
                for (cond, value) in whens {
                    let case = match operand {
                        Some(o) => {
                            Bson::Document(doc! { "$eq": [self.expr(o)?, self.expr(cond)?] })
                        }
                        None => self.expr(cond)?,
                    };
                    branches.push(Bson::Document(
                        doc! { "case": case, "then": self.expr(value)? },
                    ));
                }
                let default = match otherwise {
                    Some(o) => self.expr(o)?,
                    None => Bson::Null,
                };
                Bson::Document(doc! { "$switch": { "branches": branches, "default": default } })
            }
            Expr::Cast { expr, ty } => {
                let op = match ty.as_str() {
                    "INT" | "INTEGER" | "SMALLINT" | "TINYINT" => "$toInt",
                    "BIGINT" | "LONG" => "$toLong",
                    "FLOAT" | "REAL" | "DOUBLE" => "$toDouble",
                    "DECIMAL" | "NUMERIC" | "MONEY" => "$toDecimal",
                    "VARCHAR" | "NVARCHAR" | "CHAR" | "NCHAR" | "TEXT" | "STRING" => "$toString",
                    "DATE" | "DATETIME" | "DATETIME2" | "TIMESTAMP" => "$toDate",
                    "BIT" | "BOOL" | "BOOLEAN" => "$toBool",
                    "OBJECTID" => "$toObjectId",
                    other => return Err(format!("CAST to {other} is not supported")),
                };
                Bson::Document(doc! { op: self.expr(expr)? })
            }
            Expr::Function {
                name,
                args,
                distinct,
                star,
            } => self.function(name, args, *distinct, *star)?,
        })
    }

    fn function(&self, name: &str, args: &[Expr], distinct: bool, star: bool) -> SqlResult<Bson> {
        let upper = name.to_ascii_uppercase();
        if is_aggregate_name(&upper) {
            return Err(if self.group.is_some() {
                format!("{upper}() can't be nested inside another expression here")
            } else {
                format!("{upper}() is only allowed in SELECT, HAVING or ORDER BY")
            });
        }
        if distinct || star {
            return Err(format!("{upper}() does not take DISTINCT or *"));
        }
        let compiled = args
            .iter()
            .map(|a| self.expr(a))
            .collect::<SqlResult<Vec<_>>>()?;
        let arity = |n: std::ops::RangeInclusive<usize>| -> SqlResult<()> {
            if n.contains(&compiled.len()) {
                Ok(())
            } else {
                Err(format!(
                    "{upper}() takes {} argument(s)",
                    if n.start() == n.end() {
                        n.start().to_string()
                    } else {
                        format!("{}-{}", n.start(), n.end())
                    }
                ))
            }
        };
        let one = |op: &str| -> SqlResult<Bson> {
            arity(1..=1)?;
            Ok(Bson::Document(doc! { op: compiled[0].clone() }))
        };
        Ok(match upper.as_str() {
            "UPPER" => one("$toUpper")?,
            "LOWER" => one("$toLower")?,
            "LEN" | "LENGTH" | "CHAR_LENGTH" => one("$strLenCP")?,
            "ABS" => one("$abs")?,
            "CEILING" | "CEIL" => one("$ceil")?,
            "FLOOR" => one("$floor")?,
            "SQRT" => one("$sqrt")?,
            "YEAR" => one("$year")?,
            "MONTH" => one("$month")?,
            "DAY" => one("$dayOfMonth")?,
            "HOUR" => one("$hour")?,
            "MINUTE" => one("$minute")?,
            "SECOND" => one("$second")?,
            "TRIM" | "LTRIM" | "RTRIM" => {
                arity(1..=1)?;
                let op = match upper.as_str() {
                    "LTRIM" => "$ltrim",
                    "RTRIM" => "$rtrim",
                    _ => "$trim",
                };
                Bson::Document(doc! { op: { "input": compiled[0].clone() } })
            }
            "ROUND" => {
                arity(1..=2)?;
                Bson::Document(doc! { "$round": Bson::Array(compiled) })
            }
            "POWER" | "POW" => {
                arity(2..=2)?;
                Bson::Document(doc! { "$pow": Bson::Array(compiled) })
            }
            "SUBSTRING" | "SUBSTR" => {
                arity(3..=3)?;
                // SQL is 1-based, $substrCP is 0-based
                let start = match &compiled[1] {
                    Bson::Int32(n) => Bson::Int32(n - 1),
                    Bson::Int64(n) => Bson::Int64(n - 1),
                    other => Bson::Document(doc! { "$subtract": [other.clone(), 1] }),
                };
                Bson::Document(
                    doc! { "$substrCP": [compiled[0].clone(), start, compiled[2].clone()] },
                )
            }
            // NULLs count as '' and values are converted, like SQL Server's CONCAT
            "CONCAT" => {
                if compiled.is_empty() {
                    return Err("CONCAT() needs at least one argument".into());
                }
                let parts: Vec<Bson> = compiled
                    .into_iter()
                    .map(|p| match p {
                        Bson::String(ref s) if !s.starts_with('$') => p,
                        other => Bson::Document(doc! { "$ifNull": [{ "$toString": other }, ""] }),
                    })
                    .collect();
                Bson::Document(doc! { "$concat": parts })
            }
            "COALESCE" | "ISNULL" | "IFNULL" | "NVL" => {
                if compiled.len() < 2 {
                    return Err(format!("{upper}() needs at least two arguments"));
                }
                Bson::Document(doc! { "$ifNull": compiled })
            }
            "NOW" | "GETDATE" | "SYSDATETIME" | "GETUTCDATE" => {
                arity(0..=0)?;
                Bson::String("$$NOW".into())
            }
            other => return Err(format!("Unsupported function {other}()")),
        })
    }

    /// Query-language filter for `expr`, when it can be written without $expr
    /// (which keeps indexes usable). `None` means "use $expr".
    fn filter(&self, expr: &Expr) -> SqlResult<Option<Document>> {
        Ok(match expr {
            Expr::Binary(BinOp::And, l, r) => match (self.filter(l)?, self.filter(r)?) {
                (Some(a), Some(b)) => Some(doc! { "$and": [a, b] }),
                _ => None,
            },
            Expr::Binary(BinOp::Or, l, r) => match (self.filter(l)?, self.filter(r)?) {
                (Some(a), Some(b)) => Some(doc! { "$or": [a, b] }),
                _ => None,
            },
            Expr::Not(e) => self.filter(e)?.map(|inner| doc! { "$nor": [inner] }),
            Expr::Binary(op, l, r) if query_op(*op).is_some() => {
                let (path, value, op) = match (l.as_ref(), r.as_ref()) {
                    (Expr::Column(c), other) => match constant(other)? {
                        Some(v) => (self.scope.path(c), v, *op),
                        None => return Ok(None),
                    },
                    (other, Expr::Column(c)) => match constant(other)? {
                        Some(v) => (self.scope.path(c), v, flip(*op)),
                        None => return Ok(None),
                    },
                    _ => return Ok(None),
                };
                let Some(name) = query_op(op) else {
                    return Ok(None);
                };
                Some(
                    if op == BinOp::Eq
                        && !matches!(value, Bson::RegularExpression(_) | Bson::Document(_))
                    {
                        doc! { path: value }
                    } else {
                        doc! { path: { name: value } }
                    },
                )
            }
            Expr::IsNull { expr, negated } => match expr.as_ref() {
                Expr::Column(c) => Some(if *negated {
                    doc! { self.scope.path(c): { "$ne": Bson::Null } }
                } else {
                    doc! { self.scope.path(c): Bson::Null }
                }),
                _ => None,
            },
            Expr::InList {
                expr,
                list,
                negated,
            } => {
                let Expr::Column(c) = expr.as_ref() else {
                    return Ok(None);
                };
                let mut values = Vec::new();
                for item in list {
                    match constant(item)? {
                        Some(v) => values.push(v),
                        None => return Ok(None),
                    }
                }
                let op = if *negated { "$nin" } else { "$in" };
                Some(doc! { self.scope.path(c): { op: values } })
            }
            Expr::Between {
                expr,
                low,
                high,
                negated,
            } => {
                let (Expr::Column(c), Some(lo), Some(hi)) =
                    (expr.as_ref(), constant(low)?, constant(high)?)
                else {
                    return Ok(None);
                };
                let path = self.scope.path(c);
                Some(if *negated {
                    doc! { "$or": [{ path.clone(): { "$lt": lo } }, { path: { "$gt": hi } }] }
                } else {
                    doc! { path: { "$gte": lo, "$lte": hi } }
                })
            }
            Expr::Like {
                expr,
                pattern,
                negated,
                case_insensitive,
            } => {
                let (Expr::Column(c), Expr::Literal(Lit::Str(p))) =
                    (expr.as_ref(), pattern.as_ref())
                else {
                    return Ok(None);
                };
                let re = Bson::RegularExpression(like_regex(p, *case_insensitive));
                Some(if *negated {
                    doc! { self.scope.path(c): { "$not": re } }
                } else {
                    doc! { self.scope.path(c): re }
                })
            }
            // A bare boolean field
            Expr::Column(c) => Some(doc! { self.scope.path(c): true }),
            _ => None,
        })
    }

    /// $match document for a condition: query language when possible, else $expr
    fn match_doc(&self, expr: &Expr) -> SqlResult<Document> {
        if let Some(filter) = self.filter(expr)? {
            return Ok(filter);
        }
        // Mixed AND: keep the query-language parts usable by indexes
        let mut conjuncts = Vec::new();
        split_and(expr, &mut conjuncts);
        if conjuncts.len() > 1 {
            let mut parts = Vec::new();
            for c in conjuncts {
                parts.push(match self.filter(c)? {
                    Some(f) => f,
                    None => doc! { "$expr": self.expr(c)? },
                });
            }
            return Ok(doc! { "$and": parts });
        }
        Ok(doc! { "$expr": self.expr(expr)? })
    }
}

fn unreachable_literal() -> SqlResult<Bson> {
    Err("internal error: literal not converted".into())
}

fn split_and<'e>(expr: &'e Expr, out: &mut Vec<&'e Expr>) {
    if let Expr::Binary(BinOp::And, l, r) = expr {
        split_and(l, out);
        split_and(r, out);
    } else {
        out.push(expr);
    }
}

/// Field name for an output column: no dots, no leading `$`
fn sanitize(name: &str) -> String {
    let cleaned: String = name.trim_start_matches('$').replace('.', "_");
    if cleaned.is_empty() {
        "expr".to_string()
    } else {
        cleaned
    }
}

fn default_name(expr: &Expr, index: usize) -> String {
    match expr {
        Expr::Column(parts) => parts.last().cloned().unwrap_or_default(),
        Expr::Function {
            name,
            args,
            star,
            distinct,
        } => {
            let fname = name.to_ascii_lowercase();
            match (args.first(), star) {
                (_, true) => fname,
                (Some(Expr::Column(parts)), _) => format!(
                    "{fname}{}_{}",
                    if *distinct { "_distinct" } else { "" },
                    parts.join("_")
                ),
                _ => format!("{fname}{}", index + 1),
            }
        }
        _ => format!("expr{}", index + 1),
    }
}

fn unique(name: String, used: &mut Vec<String>) -> String {
    let mut candidate = name.clone();
    let mut n = 2;
    while used.iter().any(|u| u == &candidate) {
        candidate = format!("{name}_{n}");
        n += 1;
    }
    used.push(candidate.clone());
    candidate
}

fn accumulator(expr: &Expr, scope_compiler: &Compiler<'_>) -> SqlResult<(Document, AggKind)> {
    let Expr::Function {
        name,
        args,
        distinct,
        star,
    } = expr
    else {
        return Err("internal error: not an aggregate".into());
    };
    let upper = name.to_ascii_uppercase();
    let arg = || -> SqlResult<Bson> {
        match args.as_slice() {
            [a] if !contains_aggregate(a) => scope_compiler.expr(a),
            [_] => Err(format!("Nested aggregates in {upper}() are not supported")),
            _ => Err(format!("{upper}() takes one argument")),
        }
    };
    Ok(match upper.as_str() {
        "COUNT" if *star => (doc! { "$sum": 1 }, AggKind::Plain),
        "COUNT" if *distinct => (doc! { "$addToSet": arg()? }, AggKind::CountDistinct),
        // Count non-null, non-missing values
        "COUNT" => (
            doc! { "$sum": { "$cond": [{ "$gt": [arg()?, Bson::Null] }, 1, 0] } },
            AggKind::Plain,
        ),
        _ if *distinct => return Err(format!("{upper}(DISTINCT ...) is not supported")),
        _ if *star => return Err(format!("{upper}(*) is not valid")),
        "SUM" => (doc! { "$sum": arg()? }, AggKind::Plain),
        "AVG" => (doc! { "$avg": arg()? }, AggKind::Plain),
        "MIN" => (doc! { "$min": arg()? }, AggKind::Plain),
        "MAX" => (doc! { "$max": arg()? }, AggKind::Plain),
        "STDEV" | "STDDEV" | "STDDEV_SAMP" => (doc! { "$stdDevSamp": arg()? }, AggKind::Plain),
        "STDEVP" | "STDDEV_POP" => (doc! { "$stdDevPop": arg()? }, AggKind::Plain),
        other => return Err(format!("Unsupported aggregate {other}()")),
    })
}

/// Compile a parsed query to a pipeline on its FROM collection
pub fn compile_query(query: &Query) -> SqlResult<Compiled> {
    let mut scope = Scope::new(&query.from);
    let mut pipeline: Vec<Document> = Vec::new();

    // --- JOINs → $lookup + $unwind ---------------------------------------------
    let mut join_stages: Vec<Document> = Vec::new();
    let mut post_join_filters: Vec<Expr> = Vec::new();
    for join in &query.joins {
        let field = sanitize(join.table.alias.as_deref().unwrap_or(&join.table.name));
        let mut names = vec![join.table.name.clone()];
        names.extend(join.table.alias.clone());
        let join_index = scope.joins.len();
        scope.joins.push(JoinScope {
            names,
            field: field.clone(),
        });

        let mut conjuncts = Vec::new();
        split_and(&join.on, &mut conjuncts);
        let mut key: Option<(String, String)> = None;
        for c in conjuncts {
            if key.is_none() {
                if let Expr::Binary(BinOp::Eq, l, r) = c {
                    if let (Expr::Column(a), Expr::Column(b)) = (l.as_ref(), r.as_ref()) {
                        let (pa, ja) = scope.resolve(a);
                        let (pb, jb) = scope.resolve(b);
                        let prefix = format!("{field}.");
                        let local_foreign = match (ja == Some(join_index), jb == Some(join_index)) {
                            (false, true) => Some((pa, pb)),
                            (true, false) => Some((pb, pa)),
                            _ => None,
                        };
                        if let Some((local, foreign)) = local_foreign {
                            key = Some((
                                local,
                                foreign
                                    .strip_prefix(&prefix)
                                    .unwrap_or(&foreign)
                                    .to_string(),
                            ));
                            continue;
                        }
                    }
                }
            }
            if join.left {
                return Err("LEFT JOIN ... ON supports a single equality (a.x = b.y); move other conditions to WHERE".into());
            }
            post_join_filters.push(c.clone());
        }
        let Some((local, foreign)) = key else {
            return Err(format!(
                "JOIN {} needs an equality between the two collections, e.g. ON a.id = b.a_id",
                join.table.name
            ));
        };
        join_stages.push(doc! { "$lookup": { "from": join.table.name.clone(), "localField": local, "foreignField": foreign, "as": field.clone() } });
        join_stages.push(doc! { "$unwind": { "path": format!("${field}"), "preserveNullAndEmptyArrays": join.left } });
    }

    let plain = Compiler { scope, group: None };

    // --- WHERE ---------------------------------------------------------------------
    let mut pre_join = Vec::new();
    let mut post_join = post_join_filters;
    if let Some(filter) = &query.filter {
        if contains_aggregate(filter) {
            return Err("Aggregates are not allowed in WHERE — use HAVING".into());
        }
        let mut conjuncts = Vec::new();
        split_and(filter, &mut conjuncts);
        for c in conjuncts {
            // Conditions on the base collection run before the lookups
            let mut uses_join = false;
            walk(c, &mut |e| {
                if let Expr::Column(parts) = e {
                    if plain.scope.resolve(parts).1.is_some() {
                        uses_join = true;
                    }
                }
            });
            if uses_join {
                post_join.push(c.clone())
            } else {
                pre_join.push(c.clone())
            }
        }
    }
    if let Some(cond) = and_all(pre_join) {
        pipeline.push(doc! { "$match": plain.match_doc(&cond)? });
    }
    pipeline.extend(join_stages);
    if let Some(cond) = and_all(post_join) {
        pipeline.push(doc! { "$match": plain.match_doc(&cond)? });
    }

    let has_wildcard = query
        .items
        .iter()
        .any(|i| matches!(i, SelectItem::Wildcard));
    let is_aggregate = !query.group_by.is_empty()
        || query.having.is_some()
        || query
            .items
            .iter()
            .any(|i| matches!(i, SelectItem::Expr { expr, .. } if contains_aggregate(expr)));

    // Output names
    let mut used = Vec::new();
    let mut outputs: Vec<(String, &Expr)> = Vec::new();
    for (index, item) in query.items.iter().enumerate() {
        if let SelectItem::Expr { expr, alias } = item {
            let name = unique(
                sanitize(&alias.clone().unwrap_or_else(|| default_name(expr, index))),
                &mut used,
            );
            outputs.push((name, expr));
        }
    }

    if is_aggregate || query.distinct {
        if has_wildcard {
            return Err("SELECT * can't be combined with GROUP BY, DISTINCT or aggregates".into());
        }
        compile_grouped(query, &plain, &outputs, &mut pipeline)?;
    } else {
        compile_plain(query, &plain, &outputs, has_wildcard, &mut pipeline)?;
    }

    Ok(Compiled {
        collection: query.from.name.clone(),
        pipeline,
        columns: (!has_wildcard).then(|| outputs.iter().map(|(name, _)| name.clone()).collect()),
    })
}

fn and_all(mut exprs: Vec<Expr>) -> Option<Expr> {
    let first = if exprs.is_empty() {
        return None;
    } else {
        exprs.remove(0)
    };
    Some(exprs.into_iter().fold(first, |acc, e| {
        Expr::Binary(BinOp::And, Box::new(acc), Box::new(e))
    }))
}

fn push_paging(query: &Query, pipeline: &mut Vec<Document>) {
    if let Some(offset) = query.offset {
        pipeline.push(doc! { "$skip": i64::try_from(offset).unwrap_or(i64::MAX) });
    }
    if let Some(limit) = query.limit {
        pipeline.push(doc! { "$limit": i64::try_from(limit).unwrap_or(i64::MAX) });
    }
}

/// Projection value for an output: `1`-style paths stay readable, literals are wrapped
fn projection_value(compiled: Bson) -> Bson {
    match compiled {
        Bson::String(s) if s.starts_with('$') => Bson::String(s),
        Bson::Document(d) => Bson::Document(d),
        other => Bson::Document(doc! { "$literal": other }),
    }
}

fn compile_plain(
    query: &Query,
    compiler: &Compiler<'_>,
    outputs: &[(String, &Expr)],
    has_wildcard: bool,
    pipeline: &mut Vec<Document>,
) -> SqlResult<()> {
    // ORDER BY runs before the projection so unselected fields can be used;
    // aliases map back to their expressions, computed keys go to temp fields.
    let mut sort = Document::new();
    let mut temps = Document::new();
    for (index, (expr, desc)) in query.order_by.iter().enumerate() {
        if contains_aggregate(expr) {
            return Err("ORDER BY uses an aggregate but the query has no GROUP BY".into());
        }
        let target = match expr {
            Expr::Column(parts) if parts.len() == 1 => outputs
                .iter()
                .find(|(name, e)| name == &parts[0] && !matches!(e, Expr::Column(p) if p == parts))
                .map_or(expr, |(_, e)| *e),
            Expr::Literal(Lit::Int(n)) if *n >= 1 => match outputs.get(*n as usize - 1) {
                Some((_, e)) => *e,
                None => return Err(format!("ORDER BY {n} is out of range")),
            },
            _ => expr,
        };
        let key = match compiler.expr(target)? {
            Bson::String(s) if s.starts_with('$') && !s.starts_with("$$") => s[1..].to_string(),
            computed => {
                let temp = format!("__sort{index}");
                temps.insert(temp.clone(), computed);
                temp
            }
        };
        sort.insert(key, if *desc { -1 } else { 1 });
    }
    let temp_names: Vec<String> = temps.keys().cloned().collect();
    if !temps.is_empty() {
        pipeline.push(doc! { "$addFields": temps });
    }
    if !sort.is_empty() {
        pipeline.push(doc! { "$sort": sort });
    }
    push_paging(query, pipeline);

    if has_wildcard {
        let mut extra = Document::new();
        for (name, expr) in outputs {
            extra.insert(name.clone(), projection_value(compiler.expr(expr)?));
        }
        if !extra.is_empty() {
            pipeline.push(doc! { "$addFields": extra });
        }
        if !temp_names.is_empty() {
            pipeline.push(doc! { "$unset": temp_names });
        }
    } else {
        let mut project = Document::new();
        if !outputs.iter().any(|(name, _)| name == "_id") {
            project.insert("_id", 0);
        }
        for (name, expr) in outputs {
            project.insert(name.clone(), projection_value(compiler.expr(expr)?));
        }
        pipeline.push(doc! { "$project": project });
    }
    Ok(())
}

fn compile_grouped(
    query: &Query,
    plain: &Compiler<'_>,
    outputs: &[(String, &Expr)],
    pipeline: &mut Vec<Document>,
) -> SqlResult<()> {
    // Group keys: GROUP BY expressions, or every output for DISTINCT
    let mut used_keys = Vec::new();
    let key_exprs: Vec<Expr> = if query.distinct && query.group_by.is_empty() {
        if outputs.iter().any(|(_, e)| contains_aggregate(e)) {
            outputs
                .iter()
                .filter(|(_, e)| !contains_aggregate(e))
                .map(|(_, e)| (*e).clone())
                .collect()
        } else {
            outputs.iter().map(|(_, e)| (*e).clone()).collect()
        }
    } else {
        query.group_by.clone()
    };
    let mut keys: Vec<(Expr, String)> = Vec::new();
    for (i, key) in key_exprs.iter().enumerate() {
        if contains_aggregate(key) {
            return Err("GROUP BY can't contain aggregates".into());
        }
        let name = outputs
            .iter()
            .find(|(_, e)| plain.same(e, key))
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| format!("k{i}"));
        keys.push((key.clone(), unique(sanitize(&name), &mut used_keys)));
    }

    // Every distinct aggregate used in SELECT, HAVING and ORDER BY
    let mut agg_exprs: Vec<Expr> = Vec::new();
    let mut collect = |expr: &Expr| {
        walk(expr, &mut |e| {
            if let Expr::Function { name, .. } = e {
                if is_aggregate_name(name) && !agg_exprs.contains(e) {
                    agg_exprs.push(e.clone());
                }
            }
        });
    };
    outputs.iter().for_each(|(_, e)| collect(e));
    if let Some(h) = &query.having {
        collect(h);
    }
    query.order_by.iter().for_each(|(e, _)| collect(e));

    let mut group_doc = Document::new();
    group_doc.insert(
        "_id",
        if keys.is_empty() {
            Bson::Null
        } else {
            let mut id = Document::new();
            for (expr, name) in &keys {
                id.insert(name.clone(), plain.expr(expr)?);
            }
            Bson::Document(id)
        },
    );
    let mut aggregates: Vec<(Expr, String, AggKind)> = Vec::new();
    for (i, expr) in agg_exprs.iter().enumerate() {
        let (acc, kind) = accumulator(expr, plain)?;
        let name = format!("__agg{i}");
        group_doc.insert(name.clone(), acc);
        aggregates.push((expr.clone(), name, kind));
    }
    pipeline.push(doc! { "$group": group_doc });

    let grouped = Compiler {
        scope: Scope {
            base: plain.scope.base.clone(),
            joins: plain
                .scope
                .joins
                .iter()
                .map(|j| JoinScope {
                    names: j.names.clone(),
                    field: j.field.clone(),
                })
                .collect(),
        },
        group: Some(GroupContext {
            keys: &keys,
            aggregates: &aggregates,
        }),
    };

    if let Some(having) = &query.having {
        pipeline.push(doc! { "$match": { "$expr": grouped.expr(having)? } });
    }

    let mut project = Document::new();
    project.insert("_id", 0);
    for (name, expr) in outputs {
        project.insert(name.clone(), projection_value(grouped.expr(expr)?));
    }
    pipeline.push(doc! { "$project": project });

    // ORDER BY after grouping refers to output columns
    let mut sort = Document::new();
    for (expr, desc) in &query.order_by {
        let name = match expr {
            Expr::Column(parts)
                if parts.len() == 1 && outputs.iter().any(|(n, _)| n == &parts[0]) =>
            {
                parts[0].clone()
            }
            Expr::Literal(Lit::Int(n)) if *n >= 1 => match outputs.get(*n as usize - 1) {
                Some((name, _)) => name.clone(),
                None => return Err(format!("ORDER BY {n} is out of range")),
            },
            _ => match outputs.iter().find(|(_, e)| grouped.same(e, expr)) {
                Some((name, _)) => name.clone(),
                None => return Err(
                    "In grouped queries, ORDER BY must use a selected column, alias or position"
                        .into(),
                ),
            },
        };
        sort.insert(name, if *desc { -1 } else { 1 });
    }
    if !sort.is_empty() {
        pipeline.push(doc! { "$sort": sort });
    }
    push_paging(query, pipeline);
    Ok(())
}

/// Parse and compile in one step
pub fn compile(src: &str) -> SqlResult<Compiled> {
    let query = parse(src)?;
    compile_query(&query)
}

// ============================================================================
// Shell printer
// ============================================================================

fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

/// mongo-shell literal for a BSON value (re-parseable by `shell.rs`)
pub fn shell_value(value: &Bson) -> String {
    match value {
        Bson::Null => "null".into(),
        Bson::Undefined => "undefined".into(),
        Bson::Boolean(b) => b.to_string(),
        Bson::Int32(v) => v.to_string(),
        Bson::Int64(v) => format!("NumberLong(\"{v}\")"),
        Bson::Double(v) if v.is_nan() => "NaN".into(),
        Bson::Double(v) if v.is_infinite() => {
            if *v > 0.0 {
                "Infinity".into()
            } else {
                "-Infinity".into()
            }
        }
        Bson::Double(v) if v.fract() == 0.0 && v.abs() < 1e15 => format!("{v:.1}"),
        Bson::Double(v) => v.to_string(),
        Bson::String(s) => quote(s),
        Bson::ObjectId(oid) => format!("ObjectId(\"{}\")", oid.to_hex()),
        Bson::DateTime(dt) => format!(
            "ISODate(\"{}\")",
            dt.try_to_rfc3339_string()
                .unwrap_or_else(|_| dt.timestamp_millis().to_string())
        ),
        Bson::Decimal128(d) => format!("NumberDecimal(\"{d}\")"),
        Bson::RegularExpression(r) => {
            let mut escaped = String::new();
            let mut prev_backslash = false;
            for c in r.pattern.chars() {
                if c == '/' && !prev_backslash {
                    escaped.push('\\');
                }
                prev_backslash = c == '\\' && !prev_backslash;
                escaped.push(c);
            }
            format!("/{escaped}/{}", r.options)
        }
        Bson::Timestamp(t) => format!("Timestamp({}, {})", t.time, t.increment),
        Bson::MinKey => "MinKey()".into(),
        Bson::MaxKey => "MaxKey()".into(),
        Bson::Array(items) => format!(
            "[{}]",
            items.iter().map(shell_value).collect::<Vec<_>>().join(", ")
        ),
        Bson::Document(doc) => {
            if doc.is_empty() {
                return "{}".into();
            }
            let fields: Vec<String> = doc
                .iter()
                .map(|(k, v)| {
                    let key = if is_identifier(k) {
                        k.clone()
                    } else {
                        quote(k)
                    };
                    format!("{key}: {}", shell_value(v))
                })
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        other => quote(&other.clone().into_relaxed_extjson().to_string()),
    }
}

/// `db.coll.aggregate([...])`, one stage per line
pub fn shell_pipeline(compiled: &Compiled) -> String {
    let collection = if is_identifier(&compiled.collection) && !compiled.collection.starts_with('$')
    {
        format!("db.{}", compiled.collection)
    } else {
        format!("db.getCollection({})", quote(&compiled.collection))
    };
    if compiled.pipeline.is_empty() {
        return format!("{collection}.aggregate([])");
    }
    let stages: Vec<String> = compiled
        .pipeline
        .iter()
        .map(|stage| format!("  {}", shell_value(&Bson::Document(stage.clone()))))
        .collect();
    format!("{collection}.aggregate([\n{}\n])", stages.join(",\n"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::db::mongodb::shell::{parse_single, Statement};

    fn pipeline(sql: &str) -> Vec<Document> {
        compile(sql)
            .unwrap_or_else(|e| panic!("{e}\n{sql}"))
            .pipeline
    }

    fn err(sql: &str) -> String {
        compile(sql).unwrap_err()
    }

    #[test]
    fn simple_select_with_filter_sort_and_limit() {
        let c = compile("SELECT name, age AS years FROM users WHERE age >= 30 AND status = 'active' ORDER BY age DESC LIMIT 10").unwrap();
        assert_eq!(c.collection, "users");
        assert_eq!(
            c.columns,
            Some(vec!["name".to_string(), "years".to_string()])
        );
        assert_eq!(
            compile("SELECT *, a + 1 AS b FROM t").unwrap().columns,
            None
        );
        assert_eq!(
            c.pipeline,
            vec![
                doc! { "$match": { "$and": [{ "age": { "$gte": 30 } }, { "status": "active" }] } },
                doc! { "$sort": { "age": -1 } },
                doc! { "$limit": 10_i64 },
                doc! { "$project": { "_id": 0, "name": "$name", "years": "$age" } },
            ]
        );
    }

    #[test]
    fn select_star_top_and_tsql_quoting() {
        assert_eq!(
            pipeline("SELECT TOP 5 * FROM [order-items] o WHERE o.[unit price] > 2.5"),
            vec![
                doc! { "$match": { "unit price": { "$gt": 2.5 } } },
                doc! { "$limit": 5_i64 }
            ]
        );
        assert_eq!(
            compile(r#"select * from "order-items""#)
                .unwrap()
                .collection,
            "order-items"
        );
    }

    #[test]
    fn nested_fields_and_aliases() {
        assert_eq!(
            pipeline("SELECT u.address.city FROM users u WHERE address.zip = '2000'"),
            vec![
                doc! { "$match": { "address.zip": "2000" } },
                doc! { "$project": { "_id": 0, "city": "$address.city" } },
            ]
        );
    }

    #[test]
    fn predicates_compile_to_query_operators() {
        let m = |sql: &str| {
            pipeline(&format!("SELECT * FROM t WHERE {sql}"))[0]
                .get_document("$match")
                .unwrap()
                .clone()
        };
        assert_eq!(
            m("a IN (1, 2) AND b NOT IN ('x')"),
            doc! { "$and": [{ "a": { "$in": [1, 2] } }, { "b": { "$nin": ["x"] } }] }
        );
        assert_eq!(
            m("a IS NULL OR b IS NOT NULL"),
            doc! { "$or": [{ "a": Bson::Null }, { "b": { "$ne": Bson::Null } }] }
        );
        assert_eq!(
            m("a BETWEEN 1 AND 5"),
            doc! { "a": { "$gte": 1, "$lte": 5 } }
        );
        assert_eq!(m("5 < a"), doc! { "a": { "$gt": 5 } });
        assert_eq!(m("a <> 3"), doc! { "a": { "$ne": 3 } });
        assert_eq!(m("NOT (a = 1)"), doc! { "$nor": [{ "a": 1 }] });
        assert_eq!(m("active"), doc! { "active": true });
        assert_eq!(
            m("name LIKE 'Jo%n_'"),
            doc! { "name": Bson::RegularExpression(Regex { pattern: "^Jo.*n.$".into(), options: "s".into() }) }
        );
        assert_eq!(
            m("name ILIKE '%a.b%'"),
            doc! { "name": Bson::RegularExpression(Regex { pattern: "^.*a\\.b.*$".into(), options: "is".into() }) }
        );
    }

    #[test]
    fn typed_literals_and_shell_helpers() {
        let m = pipeline("SELECT * FROM t WHERE _id = ObjectId('65a1f0c2e4b0a1b2c3d4e5f6') AND created >= DATE '2024-01-01' AND n = NumberLong(5)");
        let flat = shell_value(&Bson::Document(m[0].clone()));
        assert!(
            flat.contains("ObjectId(\"65a1f0c2e4b0a1b2c3d4e5f6\")"),
            "{flat}"
        );
        assert!(flat.contains("ISODate(\"2024-01-01T00:00:00Z\")"), "{flat}");
        assert!(flat.contains("NumberLong(\"5\")"), "{flat}");
    }

    #[test]
    fn field_to_field_comparisons_use_expr() {
        assert_eq!(
            pipeline("SELECT * FROM t WHERE qty > reorder_level AND kind = 'x'")[0],
            doc! { "$match": { "$and": [{ "$expr": { "$gt": ["$qty", "$reorder_level"] } }, { "kind": "x" }] } }
        );
    }

    #[test]
    fn computed_columns_and_functions() {
        let p = pipeline("SELECT UPPER(name) AS n, price * qty total, CONCAT(first, ' ', last) AS full_name, SUBSTRING(code, 2, 3) c, COALESCE(nick, name) display, 1 AS one FROM t");
        assert_eq!(
            p[0],
            doc! { "$project": {
                "_id": 0,
                "n": { "$toUpper": "$name" },
                "total": { "$multiply": ["$price", "$qty"] },
                "full_name": { "$concat": [{ "$ifNull": [{ "$toString": "$first" }, ""] }, " ", { "$ifNull": [{ "$toString": "$last" }, ""] }] },
                "c": { "$substrCP": ["$code", 1, 3] },
                "display": { "$ifNull": ["$nick", "$name"] },
                "one": { "$literal": 1 },
            } }
        );
    }

    #[test]
    fn case_and_cast() {
        let p = pipeline("SELECT CASE WHEN age >= 18 THEN 'adult' ELSE 'minor' END AS band, CAST(code AS INT) n FROM t");
        assert_eq!(
            p[0],
            doc! { "$project": {
                "_id": 0,
                "band": { "$switch": { "branches": [{ "case": { "$gte": ["$age", 18] }, "then": "adult" }], "default": "minor" } },
                "n": { "$toInt": "$code" },
            } }
        );
    }

    #[test]
    fn group_by_with_aggregates_having_and_order() {
        let p = pipeline(
            "SELECT country, COUNT(*) AS n, AVG(age) avg_age, COUNT(DISTINCT city) cities FROM users WHERE active = true GROUP BY country HAVING COUNT(*) > 1 ORDER BY n DESC",
        );
        assert_eq!(
            p,
            vec![
                doc! { "$match": { "active": true } },
                doc! { "$group": {
                    "_id": { "country": "$country" },
                    "__agg0": { "$sum": 1 },
                    "__agg1": { "$avg": "$age" },
                    "__agg2": { "$addToSet": "$city" },
                } },
                doc! { "$match": { "$expr": { "$gt": ["$__agg0", 1] } } },
                doc! { "$project": {
                    "_id": 0,
                    "country": "$_id.country",
                    "n": "$__agg0",
                    "avg_age": "$__agg1",
                    "cities": { "$size": { "$filter": { "input": "$__agg2", "cond": { "$gt": ["$$this", Bson::Null] } } } },
                } },
                doc! { "$sort": { "n": -1 } },
            ]
        );
    }

    #[test]
    fn aggregates_without_group_by_and_default_names() {
        assert_eq!(
            pipeline("SELECT COUNT(*), SUM(total), MAX(created) FROM orders"),
            vec![
                doc! { "$group": { "_id": Bson::Null, "__agg0": { "$sum": 1 }, "__agg1": { "$sum": "$total" }, "__agg2": { "$max": "$created" } } },
                doc! { "$project": { "_id": 0, "count": "$__agg0", "sum_total": "$__agg1", "max_created": "$__agg2" } },
            ]
        );
    }

    #[test]
    fn distinct_and_count_of_column() {
        assert_eq!(
            pipeline("SELECT DISTINCT country FROM users ORDER BY country"),
            vec![
                doc! { "$group": { "_id": { "country": "$country" } } },
                doc! { "$project": { "_id": 0, "country": "$_id.country" } },
                doc! { "$sort": { "country": 1 } },
            ]
        );
        assert_eq!(
            pipeline("SELECT COUNT(email) FROM users")[0],
            doc! { "$group": { "_id": Bson::Null, "__agg0": { "$sum": { "$cond": [{ "$gt": ["$email", Bson::Null] }, 1, 0] } } } }
        );
    }

    #[test]
    fn joins_become_lookups() {
        let p = pipeline(
            "SELECT o.total, c.name AS customer FROM orders o LEFT JOIN customers c ON o.customerId = c._id WHERE o.total > 100 AND c.country = 'AU'",
        );
        assert_eq!(
            p,
            vec![
                doc! { "$match": { "total": { "$gt": 100 } } },
                doc! { "$lookup": { "from": "customers", "localField": "customerId", "foreignField": "_id", "as": "c" } },
                doc! { "$unwind": { "path": "$c", "preserveNullAndEmptyArrays": true } },
                doc! { "$match": { "c.country": "AU" } },
                doc! { "$project": { "_id": 0, "total": "$total", "customer": "$c.name" } },
            ]
        );
        // Reversed ON sides and an inner join
        let p =
            pipeline("SELECT * FROM orders JOIN customers ON customers._id = orders.customerId");
        assert_eq!(
            p[0],
            doc! { "$lookup": { "from": "customers", "localField": "customerId", "foreignField": "_id", "as": "customers" } }
        );
        assert_eq!(
            p[1],
            doc! { "$unwind": { "path": "$customers", "preserveNullAndEmptyArrays": false } }
        );
    }

    #[test]
    fn order_by_unselected_alias_and_computed_keys() {
        let p = pipeline("SELECT name FROM t ORDER BY created DESC, LEN(name) OFFSET 10 ROWS FETCH NEXT 5 ROWS ONLY");
        assert_eq!(
            p,
            vec![
                doc! { "$addFields": { "__sort1": { "$strLenCP": "$name" } } },
                doc! { "$sort": { "created": -1, "__sort1": 1 } },
                doc! { "$skip": 10_i64 },
                doc! { "$limit": 5_i64 },
                doc! { "$project": { "_id": 0, "name": "$name" } },
            ]
        );
        // Alias of a field sorts by the field
        assert_eq!(
            pipeline("SELECT age AS years FROM t ORDER BY years")[0],
            doc! { "$sort": { "age": 1 } }
        );
        // SELECT * removes temp sort fields
        let p = pipeline("SELECT * FROM t ORDER BY a + b");
        assert_eq!(p.last().unwrap(), &doc! { "$unset": ["__sort0"] });
    }

    #[test]
    fn rejects_unsupported_sql_with_clear_messages() {
        assert!(err("UPDATE t SET a = 1").contains("SELECT only"));
        assert!(err("SELECT * FROM t WHERE a IN (SELECT b FROM u)").contains("Subqueries"));
        assert!(err("SELECT a FROM t UNION SELECT a FROM u").contains("UNION"));
        assert!(err("SELECT * FROM a RIGHT JOIN b ON a.x = b.y").contains("INNER and LEFT"));
        assert!(err("SELECT * FROM a, b").contains("Comma joins"));
        assert!(err("SELECT name, COUNT(*) FROM t").contains("GROUP BY"));
        assert!(err("SELECT * FROM t WHERE COUNT(*) > 1").contains("HAVING"));
        assert!(err("SELECT * FROM t GROUP BY a").contains("SELECT *"));
        assert!(err("SELECT FOO(a) FROM t").contains("Unsupported function FOO"));
        assert!(err("SELECT * FROM a JOIN b ON a.x > b.y").contains("equality"));
        let msg = err("SELECT * FROM t WHERE");
        assert!(msg.starts_with("SQL error at line 1, column 22"), "{msg}");
    }

    #[test]
    fn printed_pipeline_parses_back_to_the_same_pipeline() {
        for sql in [
            "SELECT name, age FROM users WHERE name LIKE 'a/b%' AND age BETWEEN 1 AND 9 ORDER BY age",
            "SELECT country, COUNT(*) n, AVG(score) FROM [user stats] GROUP BY country HAVING AVG(score) > 2.5",
            "SELECT o.total, c.name FROM orders o JOIN customers c ON o.cid = c._id WHERE created >= DATE '2024-02-03T04:05:06Z' AND _id <> ObjectId('65a1f0c2e4b0a1b2c3d4e5f6')",
            "SELECT * FROM t WHERE big = 5000000000 AND f = 2.0 AND s = 'it''s \"quoted\"' AND tag = '$notAField'",
        ] {
            let compiled = compile(sql).unwrap();
            let text = shell_pipeline(&compiled);
            let Statement::CollectionCall { collection, call, .. } = parse_single(&text).unwrap_or_else(|e| panic!("{}\n{text}", e.describe(&text))) else {
                panic!("expected a collection call: {text}");
            };
            assert_eq!(collection, compiled.collection);
            assert_eq!(call.name, "aggregate");
            let reparsed: Vec<Document> = match convert::to_bson(&call.args[0]).unwrap() {
                Bson::Array(stages) => stages.into_iter().map(|s| s.as_document().unwrap().clone()).collect(),
                other => panic!("{other:?}"),
            };
            assert_eq!(reparsed, compiled.pipeline, "\n{text}");
        }
    }

    #[test]
    fn non_identifier_collections_print_with_get_collection() {
        let text = shell_pipeline(&compile("SELECT * FROM [order-items]").unwrap());
        assert_eq!(text, "db.getCollection(\"order-items\").aggregate([])");
    }
}
