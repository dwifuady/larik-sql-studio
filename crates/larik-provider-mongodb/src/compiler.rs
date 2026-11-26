use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result, bail};
use mongodb::bson::{Bson, DateTime as BsonDateTime, Document, Regex, doc};

use crate::commands::MongoCommand;
use crate::formatter::{
    eval_bson_constructor, parse_date_string, parse_object_id_bson, parse_uuid_bson,
};

/// Compiles Larik T-SQL statements (`SELECT`, `INSERT`, `UPDATE`, `DELETE`, `USE`)
/// into native MongoDB aggregation pipelines and CRUD operations.
pub struct TSqlMongoCompiler;

impl TSqlMongoCompiler {
    /// Compile a single SQL statement without schema type hints.
    pub fn compile(sql: &str) -> Result<MongoCommand> {
        Self::compile_with_field_types(sql, &HashMap::new())
    }

    /// Compile a single SQL statement with sampled collection field types for automatic
    /// `uuid` and `objectId` literal coercion.
    pub fn compile_with_field_types(
        sql: &str,
        field_types: &HashMap<String, String>,
    ) -> Result<MongoCommand> {
        let tokens = tokenize(sql)?;
        if tokens.is_empty() {
            bail!("Empty SQL statement.");
        }
        let mut parser = Parser::new(tokens, field_types);
        let cmd = parser.parse_statement()?;
        parser.skip_trailing_semicolons();
        if !parser.is_eof() {
            bail!(
                "Unexpected token '{}' after SQL statement.",
                parser.peek_display()
            );
        }
        Ok(cmd)
    }

    /// Best-effort extraction of the primary collection name referenced by a SQL statement,
    /// used to look up cached sampled field types before full compilation.
    pub fn find_primary_collection(sql: &str) -> Option<String> {
        let tokens = tokenize(sql).ok()?;
        let mut i = 0;
        while i < tokens.len() {
            if let Token::Word(w) = &tokens[i] {
                if w.eq_ignore_ascii_case("FROM")
                    || w.eq_ignore_ascii_case("INTO")
                    || w.eq_ignore_ascii_case("UPDATE")
                {
                    if let Some(tok) = tokens.get(i + 1) {
                        return token_ident(tok).map(str::to_string);
                    }
                }
            }
            i += 1;
        }
        None
    }
}

// ============================================================================
// Lexer
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    QuotedIdent(String),
    StringLit(String),
    IntLit(i64),
    FloatLit(f64),
    Comma,
    Dot,
    Star,
    Plus,
    Minus,
    Slash,
    Percent,
    LParen,
    RParen,
    Semicolon,
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
}

impl Token {
    fn display(&self) -> String {
        match self {
            Token::Word(s) | Token::QuotedIdent(s) => s.clone(),
            Token::StringLit(s) => format!("'{s}'"),
            Token::IntLit(n) => n.to_string(),
            Token::FloatLit(f) => f.to_string(),
            Token::Comma => ",".into(),
            Token::Dot => ".".into(),
            Token::Star => "*".into(),
            Token::Plus => "+".into(),
            Token::Minus => "-".into(),
            Token::Slash => "/".into(),
            Token::Percent => "%".into(),
            Token::LParen => "(".into(),
            Token::RParen => ")".into(),
            Token::Semicolon => ";".into(),
            Token::Eq => "=".into(),
            Token::Neq => "<>".into(),
            Token::Lt => "<".into(),
            Token::Lte => "<=".into(),
            Token::Gt => ">".into(),
            Token::Gte => ">=".into(),
        }
    }
}

fn token_ident(tok: &Token) -> Option<&str> {
    match tok {
        Token::Word(s) | Token::QuotedIdent(s) => Some(s.as_str()),
        _ => None,
    }
}

fn tokenize(sql: &str) -> Result<Vec<Token>> {
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut out = Vec::new();

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Line comments: -- or //
        if (b == b'-' && i + 1 < len && bytes[i + 1] == b'-')
            || (b == b'/' && i + 1 < len && bytes[i + 1] == b'/')
        {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment /* ... */
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut closed = false;
            while i + 1 < len {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                bail!("Unterminated block comment in SQL statement.");
            }
            continue;
        }
        // N'...' or '...'
        if b == b'\'' || ((b == b'N' || b == b'n') && i + 1 < len && bytes[i + 1] == b'\'') {
            if b != b'\'' {
                i += 1;
            }
            i += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < len {
                if bytes[i] == b'\'' {
                    if i + 1 < len && bytes[i + 1] == b'\'' {
                        s.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                let ch = sql[i..].chars().next().unwrap();
                s.push(ch);
                i += ch.len_utf8();
            }
            if !closed {
                bail!("Unterminated string literal in SQL statement.");
            }
            out.push(Token::StringLit(s));
            continue;
        }
        // Bracketed identifier [...]
        if b == b'[' {
            i += 1;
            let mut ident = String::new();
            let mut closed = false;
            while i < len {
                if bytes[i] == b']' {
                    if i + 1 < len && bytes[i + 1] == b']' {
                        ident.push(']');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                let ch = sql[i..].chars().next().unwrap();
                ident.push(ch);
                i += ch.len_utf8();
            }
            if !closed {
                bail!("Unterminated bracketed identifier '['.");
            }
            out.push(Token::QuotedIdent(ident));
            continue;
        }
        // Double-quoted or backtick identifier
        if b == b'"' || b == b'`' {
            let q = b;
            i += 1;
            let mut ident = String::new();
            let mut closed = false;
            while i < len {
                if bytes[i] == q {
                    if i + 1 < len && bytes[i + 1] == q {
                        ident.push(q as char);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                let ch = sql[i..].chars().next().unwrap();
                ident.push(ch);
                i += ch.len_utf8();
            }
            if !closed {
                bail!("Unterminated quoted identifier.");
            }
            out.push(Token::QuotedIdent(ident));
            continue;
        }

        // Two-character operators
        if i + 1 < len {
            let pair = (b, bytes[i + 1]);
            match pair {
                (b'<', b'>') | (b'!', b'=') => {
                    out.push(Token::Neq);
                    i += 2;
                    continue;
                }
                (b'<', b'=') => {
                    out.push(Token::Lte);
                    i += 2;
                    continue;
                }
                (b'>', b'=') => {
                    out.push(Token::Gte);
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }

        match b {
            b',' => {
                out.push(Token::Comma);
                i += 1;
            }
            b'.' => {
                out.push(Token::Dot);
                i += 1;
            }
            b'*' => {
                out.push(Token::Star);
                i += 1;
            }
            b'+' => {
                out.push(Token::Plus);
                i += 1;
            }
            b'-' => {
                out.push(Token::Minus);
                i += 1;
            }
            b'/' => {
                out.push(Token::Slash);
                i += 1;
            }
            b'%' => {
                out.push(Token::Percent);
                i += 1;
            }
            b'(' => {
                out.push(Token::LParen);
                i += 1;
            }
            b')' => {
                out.push(Token::RParen);
                i += 1;
            }
            b';' => {
                out.push(Token::Semicolon);
                i += 1;
            }
            b'=' => {
                out.push(Token::Eq);
                i += 1;
            }
            b'<' => {
                out.push(Token::Lt);
                i += 1;
            }
            b'>' => {
                out.push(Token::Gt);
                i += 1;
            }
            b'0'..=b'9' => {
                let start = i;
                let mut is_float = false;
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i < len && bytes[i] == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit() {
                    is_float = true;
                    i += 1;
                    while i < len && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < len && matches!(bytes[i], b'e' | b'E') {
                    is_float = true;
                    i += 1;
                    if i < len && matches!(bytes[i], b'+' | b'-') {
                        i += 1;
                    }
                    while i < len && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let raw = &sql[start..i];
                if is_float {
                    let f: f64 = raw
                        .parse()
                        .with_context(|| format!("Invalid numeric literal '{raw}'."))?;
                    out.push(Token::FloatLit(f));
                } else {
                    let n: i64 = raw
                        .parse()
                        .with_context(|| format!("Invalid integer literal '{raw}'."))?;
                    out.push(Token::IntLit(n));
                }
            }
            _ => {
                let ch = sql[i..].chars().next().unwrap();
                if ch.is_alphabetic() || ch == '_' || ch == '$' {
                    let start = i;
                    i += ch.len_utf8();
                    while i < len {
                        let c = sql[i..].chars().next().unwrap();
                        if c.is_alphanumeric() || c == '_' || c == '$' {
                            i += c.len_utf8();
                        } else {
                            break;
                        }
                    }
                    out.push(Token::Word(sql[start..i].to_string()));
                } else {
                    bail!("Unexpected character '{ch}' in SQL statement.");
                }
            }
        }
    }

    Ok(out)
}

// ============================================================================
// AST
// ============================================================================

#[derive(Debug, Clone)]
enum Expr {
    Literal(Bson),
    Column(Vec<String>),
    UnaryNeg(Box<Expr>),
    Not(Box<Expr>),
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
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
        pattern: String,
        negated: bool,
    },
    Call {
        name: String,
        distinct: bool,
        star: bool,
        args: Vec<Expr>,
    },
    Case {
        branches: Vec<(Expr, Expr)>,
        else_expr: Option<Box<Expr>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinOp {
    Or,
    And,
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Debug, Clone)]
enum SelectItem {
    Star,
    Expr { expr: Expr, alias: Option<String> },
}

#[derive(Debug, Clone)]
struct JoinClause {
    is_left: bool,
    collection: String,
    alias: String,
    left_path: Vec<String>,
    right_path: Vec<String>,
}

#[derive(Debug, Clone)]
struct OrderItem {
    expr: Expr,
    descending: bool,
}

// ============================================================================
// Parser
// ============================================================================

const CLAUSE_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS", "ON",
    "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "FETCH", "UNION", "SET", "VALUES", "INTO",
    "AND", "OR", "AS", "ASC", "DESC", "BY", "TOP", "DISTINCT", "WHEN", "THEN", "ELSE", "END",
];

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    field_types: &'a HashMap<String, String>,
}

impl<'a> Parser<'a> {
    fn new(tokens: Vec<Token>, field_types: &'a HashMap<String, String>) -> Self {
        Self {
            tokens,
            pos: 0,
            field_types,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn peek_display(&self) -> String {
        self.peek()
            .map(Token::display)
            .unwrap_or_else(|| "<eof>".to_string())
    }

    fn next(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    fn skip_trailing_semicolons(&mut self) {
        while matches!(self.peek(), Some(Token::Semicolon)) {
            self.pos += 1;
        }
    }

    fn check_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(w)) if w.eq_ignore_ascii_case(kw))
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.check_kw(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, kw: &str) -> Result<()> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            bail!("Expected keyword '{kw}', got '{}'.", self.peek_display())
        }
    }

    fn expect_token(&mut self, expected: &Token) -> Result<()> {
        if self.peek() == Some(expected) {
            self.pos += 1;
            Ok(())
        } else {
            bail!(
                "Expected '{}', got '{}'.",
                expected.display(),
                self.peek_display()
            )
        }
    }

    fn parse_ident(&mut self) -> Result<String> {
        match self.next() {
            Some(Token::Word(s)) | Some(Token::QuotedIdent(s)) => Ok(s),
            Some(other) => bail!("Expected identifier, got '{}'.", other.display()),
            None => bail!("Expected identifier, reached end of statement."),
        }
    }

    fn parse_ident_path(&mut self) -> Result<Vec<String>> {
        let mut parts = vec![self.parse_ident()?];
        while matches!(self.peek(), Some(Token::Dot)) {
            self.pos += 1;
            parts.push(self.parse_ident()?);
        }
        Ok(parts)
    }

    fn parse_optional_alias(&mut self) -> Result<Option<String>> {
        if self.eat_kw("AS") {
            return Ok(Some(self.parse_ident()?));
        }
        match self.peek() {
            Some(Token::QuotedIdent(_)) => Ok(Some(self.parse_ident()?)),
            Some(Token::Word(w))
                if !CLAUSE_KEYWORDS
                    .iter()
                    .any(|kw| kw.eq_ignore_ascii_case(w)) =>
            {
                Ok(Some(self.parse_ident()?))
            }
            _ => Ok(None),
        }
    }

    fn parse_non_negative_int(&mut self, context: &str) -> Result<i64> {
        match self.next() {
            Some(Token::IntLit(n)) if n >= 0 => Ok(n),
            Some(other) => bail!("Expected non-negative integer for {context}, got '{}'.", other.display()),
            None => bail!("Expected non-negative integer for {context}."),
        }
    }

    fn parse_statement(&mut self) -> Result<MongoCommand> {
        if self.eat_kw("SELECT") {
            self.parse_select()
        } else if self.eat_kw("INSERT") {
            self.parse_insert()
        } else if self.eat_kw("UPDATE") {
            self.parse_update()
        } else if self.eat_kw("DELETE") {
            self.parse_delete()
        } else if self.eat_kw("USE") {
            let db = self.parse_ident()?;
            Ok(MongoCommand::Use { database: db })
        } else {
            bail!(
                "Unsupported SQL statement starting with '{}'. MongoDB SQL supports SELECT, INSERT, UPDATE, DELETE, and USE (or use db.<collection>.<method>(...) shell syntax).",
                self.peek_display()
            )
        }
    }

    fn parse_select(&mut self) -> Result<MongoCommand> {
        let distinct = self.eat_kw("DISTINCT");
        let mut top_limit: Option<i64> = None;

        if self.eat_kw("TOP") {
            let paren = matches!(self.peek(), Some(Token::LParen));
            if paren {
                self.pos += 1;
            }
            top_limit = Some(self.parse_non_negative_int("TOP")?);
            if paren {
                self.expect_token(&Token::RParen)?;
            }
        }

        // Select list
        let mut items = Vec::new();
        loop {
            if matches!(self.peek(), Some(Token::Star)) {
                self.pos += 1;
                items.push(SelectItem::Star);
            } else if token_ident(self.peek().unwrap_or(&Token::Semicolon)).is_some()
                && matches!(self.peek_at(1), Some(Token::Dot))
                && matches!(self.peek_at(2), Some(Token::Star))
            {
                let _qual = self.parse_ident()?;
                self.pos += 2; // '.' '*'
                items.push(SelectItem::Star);
            } else {
                let expr = self.parse_expr()?;
                let alias = self.parse_optional_alias()?;
                items.push(SelectItem::Expr { expr, alias });
            }

            if matches!(self.peek(), Some(Token::Comma)) {
                self.pos += 1;
                continue;
            }
            break;
        }

        // FROM <collection> [alias]
        self.expect_kw("FROM")?;
        let from_path = self.parse_ident_path()?;
        let collection = from_path.last().cloned().unwrap_or_default();
        let table_alias = self.parse_optional_alias()?;
        let primary_alias = table_alias.unwrap_or_else(|| collection.clone());

        // Optional JOINs
        let mut joins = Vec::new();
        loop {
            let is_left = if self.eat_kw("LEFT") {
                let _ = self.eat_kw("OUTER");
                self.expect_kw("JOIN")?;
                true
            } else if self.eat_kw("INNER") {
                self.expect_kw("JOIN")?;
                false
            } else if self.eat_kw("JOIN") {
                false
            } else {
                break;
            };

            let join_path = self.parse_ident_path()?;
            let join_coll = join_path.last().cloned().unwrap_or_default();
            let join_alias = self
                .parse_optional_alias()?
                .unwrap_or_else(|| join_coll.clone());
            self.expect_kw("ON")?;
            let left_col = self.parse_ident_path()?;
            self.expect_token(&Token::Eq)?;
            let right_col = self.parse_ident_path()?;
            joins.push(JoinClause {
                is_left,
                collection: join_coll,
                alias: join_alias,
                left_path: left_col,
                right_path: right_col,
            });
        }

        // Optional WHERE
        let where_expr = if self.eat_kw("WHERE") {
            Some(self.parse_expr()?)
        } else {
            None
        };

        // Optional GROUP BY
        let mut group_by = Vec::new();
        if self.eat_kw("GROUP") {
            self.expect_kw("BY")?;
            loop {
                group_by.push(self.parse_expr()?);
                if matches!(self.peek(), Some(Token::Comma)) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }

        // Optional HAVING
        let having_expr = if self.eat_kw("HAVING") {
            Some(self.parse_expr()?)
        } else {
            None
        };

        // Optional ORDER BY
        let mut order_by = Vec::new();
        if self.eat_kw("ORDER") {
            self.expect_kw("BY")?;
            loop {
                let expr = self.parse_expr()?;
                let descending = if self.eat_kw("DESC") {
                    true
                } else {
                    let _ = self.eat_kw("ASC");
                    false
                };
                order_by.push(OrderItem { expr, descending });
                if matches!(self.peek(), Some(Token::Comma)) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }

        // Optional LIMIT / OFFSET / FETCH
        let mut limit = top_limit;
        let mut offset: Option<i64> = None;

        loop {
            if self.eat_kw("LIMIT") {
                let n = self.parse_non_negative_int("LIMIT")?;
                limit = Some(limit.map_or(n, |existing| existing.min(n)));
            } else if self.eat_kw("OFFSET") {
                offset = Some(self.parse_non_negative_int("OFFSET")?);
                let _ = self.eat_kw("ROWS") || self.eat_kw("ROW");
            } else if self.eat_kw("FETCH") {
                let _ = self.eat_kw("FIRST") || self.eat_kw("NEXT");
                let n = self.parse_non_negative_int("FETCH")?;
                let _ = self.eat_kw("ROWS") || self.eat_kw("ROW");
                let _ = self.eat_kw("ONLY");
                limit = Some(limit.map_or(n, |existing| existing.min(n)));
            } else {
                break;
            }
        }

        // Compile SELECT into MongoDB aggregation pipeline.
        let ctx = CompileCtx {
            primary_collection: &collection,
            primary_alias: &primary_alias,
            join_aliases: joins.iter().map(|j| j.alias.clone()).collect(),
            field_types: self.field_types,
        };

        let mut pipeline: Vec<Document> = Vec::new();

        // 1. JOINs ($lookup + $unwind)
        for join in &joins {
            let (local_field, foreign_field) =
                resolve_join_fields(&ctx, join, &join.left_path, &join.right_path);
            pipeline.push(doc! {
                "$lookup": {
                    "from": &join.collection,
                    "localField": local_field,
                    "foreignField": foreign_field,
                    "as": &join.alias,
                }
            });
            pipeline.push(doc! {
                "$unwind": {
                    "path": format!("${}", join.alias),
                    "preserveNullAndEmptyArrays": join.is_left,
                }
            });
        }

        // 2. WHERE ($match)
        if let Some(ref expr) = where_expr {
            let filter = compile_filter(&ctx, expr)?;
            if !filter.is_empty() {
                pipeline.push(doc! { "$match": filter });
            }
        }

        // Determine if this is an aggregate query
        let has_aggregates = !group_by.is_empty()
            || items.iter().any(|it| match it {
                SelectItem::Expr { expr, .. } => expr_contains_aggregate(expr),
                SelectItem::Star => false,
            })
            || having_expr.is_some();

        let mut explicit_columns: Vec<String> = Vec::new();
        let mut alias_map: HashMap<String, String> = HashMap::new();

        if has_aggregates {
            let mut group_id = Document::new();
            let mut group_key_exprs: Vec<(String, Expr)> = Vec::new();
            for (idx, gexpr) in group_by.iter().enumerate() {
                let key = default_expr_name(gexpr, idx);
                let agg_val = compile_agg_expr(&ctx, gexpr)?;
                group_id.insert(key.clone(), agg_val);
                group_key_exprs.push((key, gexpr.clone()));
            }

            let mut group_stage = Document::new();
            if group_id.is_empty() {
                group_stage.insert("_id", Bson::Null);
            } else {
                group_stage.insert("_id", Bson::Document(group_id));
            }

            let mut project_stage = doc! { "_id": 0 };
            let mut needs_post_distinct_size: Vec<(String, String)> = Vec::new();

            for (idx, item) in items.iter().enumerate() {
                match item {
                    SelectItem::Star => {
                        bail!("SELECT * cannot be combined with GROUP BY or aggregate functions.");
                    }
                    SelectItem::Expr { expr, alias } => {
                        let out_name = alias
                            .clone()
                            .unwrap_or_else(|| default_expr_name(expr, idx));
                        explicit_columns.push(out_name.clone());

                        if let Some((gkey, _)) = group_key_exprs
                            .iter()
                            .find(|(_, ge)| exprs_structurally_equal(ge, expr))
                        {
                            let field_ref = format!("$_id.{gkey}");
                            project_stage.insert(out_name.clone(), Bson::String(field_ref.clone()));
                            alias_map.insert(out_name.to_ascii_lowercase(), out_name.clone());
                        } else if let Some((acc_doc, is_distinct_count)) =
                            try_compile_accumulator(&ctx, expr)?
                        {
                            let acc_field = format!("__agg_{idx}");
                            group_stage.insert(acc_field.clone(), Bson::Document(acc_doc));
                            if is_distinct_count {
                                needs_post_distinct_size.push((out_name.clone(), acc_field.clone()));
                                project_stage.insert(
                                    out_name.clone(),
                                    doc! { "$size": format!("${acc_field}") },
                                );
                            } else {
                                project_stage
                                    .insert(out_name.clone(), Bson::String(format!("${acc_field}")));
                            }
                            alias_map.insert(out_name.to_ascii_lowercase(), out_name.clone());
                        } else {
                            // Expression over group keys or scalar expression
                            let compiled = compile_agg_expr_with_group(&ctx, expr, &group_key_exprs)?;
                            project_stage.insert(out_name.clone(), compiled);
                            alias_map.insert(out_name.to_ascii_lowercase(), out_name.clone());
                        }
                    }
                }
            }

            pipeline.push(doc! { "$group": group_stage });
            pipeline.push(doc! { "$project": project_stage });

            // HAVING runs after projection so aliases and aggregates are directly addressable!
            if let Some(ref h_expr) = having_expr {
                let having_ctx = CompileCtx {
                    primary_collection: &collection,
                    primary_alias: &primary_alias,
                    join_aliases: Vec::new(),
                    field_types: &HashMap::new(),
                };
                let rewritten = rewrite_having_expr(h_expr, &items);
                let h_filter = compile_filter(&having_ctx, &rewritten)?;
                if !h_filter.is_empty() {
                    pipeline.push(doc! { "$match": h_filter });
                }
            }
        } else {
            // Non-aggregate SELECT
            let has_star = items.iter().any(|i| matches!(i, SelectItem::Star));
            if !has_star {
                let mut project_stage = Document::new();
                let mut explicitly_includes_id = false;
                for (idx, item) in items.iter().enumerate() {
                    if let SelectItem::Expr { expr, alias } = item {
                        let out_name = alias
                            .clone()
                            .unwrap_or_else(|| default_expr_name(expr, idx));
                        if out_name == "_id" {
                            explicitly_includes_id = true;
                        }
                        explicit_columns.push(out_name.clone());
                        alias_map.insert(out_name.to_ascii_lowercase(), out_name.clone());
                        let compiled = compile_agg_expr(&ctx, expr)?;
                        project_stage.insert(out_name, compiled);
                    }
                }
                if !explicitly_includes_id {
                    project_stage.insert("_id", 0);
                }
                pipeline.push(doc! { "$project": project_stage });
            }

            if distinct {
                if explicit_columns.is_empty() {
                    bail!("SELECT DISTINCT requires an explicit column list in MongoDB.");
                }
                let mut distinct_id = Document::new();
                let mut post_project = doc! { "_id": 0 };
                for col in &explicit_columns {
                    distinct_id.insert(col.clone(), Bson::String(format!("${col}")));
                    post_project.insert(col.clone(), Bson::String(format!("$_id.{col}")));
                }
                pipeline.push(doc! { "$group": { "_id": distinct_id } });
                pipeline.push(doc! { "$project": post_project });
            }
        }

        // ORDER BY ($sort)
        if !order_by.is_empty() {
            let mut sort_doc = Document::new();
            for item in &order_by {
                let field = match &item.expr {
                    Expr::Column(parts) => {
                        let candidate = if parts.len() == 1 {
                            alias_map
                                .get(&parts[0].to_ascii_lowercase())
                                .cloned()
                                .unwrap_or_else(|| ctx.resolve_field(parts))
                        } else {
                            ctx.resolve_field(parts)
                        };
                        candidate
                    }
                    other => default_expr_name(other, 0),
                };
                sort_doc.insert(field, if item.descending { -1 } else { 1 });
            }
            pipeline.push(doc! { "$sort": sort_doc });
        }

        // OFFSET ($skip)
        if let Some(skip) = offset.filter(|&n| n > 0) {
            pipeline.push(doc! { "$skip": skip });
        }

        // LIMIT ($limit)
        if let Some(lim) = limit {
            pipeline.push(doc! { "$limit": lim });
        }

        Ok(MongoCommand::Select {
            collection,
            pipeline,
            explicit_columns,
            limit,
        })
    }

    fn parse_insert(&mut self) -> Result<MongoCommand> {
        let _ = self.eat_kw("INTO");
        let path = self.parse_ident_path()?;
        let collection = path.last().cloned().unwrap_or_default();

        let mut columns = Vec::new();
        if matches!(self.peek(), Some(Token::LParen)) {
            self.pos += 1;
            loop {
                columns.push(self.parse_ident_path()?.join("."));
                if matches!(self.peek(), Some(Token::Comma)) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            self.expect_token(&Token::RParen)?;
        }
        if columns.is_empty() {
            bail!("INSERT INTO [{collection}] requires an explicit column list: INSERT INTO [{collection}] (col1, col2) VALUES (...).");
        }

        self.expect_kw("VALUES")?;
        let mut documents = Vec::new();
        loop {
            self.expect_token(&Token::LParen)?;
            let mut values = Vec::new();
            loop {
                let expr = self.parse_expr()?;
                values.push(eval_constant_expr(&expr)?);
                if matches!(self.peek(), Some(Token::Comma)) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            self.expect_token(&Token::RParen)?;

            if values.len() != columns.len() {
                bail!(
                    "INSERT column count ({}) does not match value count ({}).",
                    columns.len(),
                    values.len()
                );
            }

            let mut doc = Document::new();
            for (col, val) in columns.iter().zip(values.into_iter()) {
                let coerced = coerce_value_for_field(self.field_types, col, val);
                insert_nested_field(&mut doc, col, coerced);
            }
            documents.push(doc);

            if matches!(self.peek(), Some(Token::Comma)) {
                self.pos += 1;
                continue;
            }
            break;
        }

        Ok(MongoCommand::Insert {
            collection,
            documents,
        })
    }

    fn parse_update(&mut self) -> Result<MongoCommand> {
        let path = self.parse_ident_path()?;
        let collection = path.last().cloned().unwrap_or_default();
        let alias = self
            .parse_optional_alias()?
            .unwrap_or_else(|| collection.clone());
        self.expect_kw("SET")?;

        let mut set_fields = Document::new();
        let unset_fields = Document::new();
        loop {
            let col_parts = self.parse_ident_path()?;
            let col = if col_parts.len() > 1 && col_parts[0].eq_ignore_ascii_case(&alias) {
                col_parts[1..].join(".")
            } else {
                col_parts.join(".")
            };
            self.expect_token(&Token::Eq)?;
            let val_expr = self.parse_expr()?;
            let val = eval_constant_expr(&val_expr)?;
            let coerced = coerce_value_for_field(self.field_types, &col, val);
            set_fields.insert(col, coerced);

            if matches!(self.peek(), Some(Token::Comma)) {
                self.pos += 1;
                continue;
            }
            break;
        }

        let ctx = CompileCtx {
            primary_collection: &collection,
            primary_alias: &alias,
            join_aliases: Vec::new(),
            field_types: self.field_types,
        };

        let filter = if self.eat_kw("WHERE") {
            let expr = self.parse_expr()?;
            compile_filter(&ctx, &expr)?
        } else {
            Document::new()
        };

        Ok(MongoCommand::Update {
            collection,
            filter,
            set_fields,
            unset_fields,
        })
    }

    fn parse_delete(&mut self) -> Result<MongoCommand> {
        let _ = self.eat_kw("FROM");
        let path = self.parse_ident_path()?;
        let collection = path.last().cloned().unwrap_or_default();
        let alias = self
            .parse_optional_alias()?
            .unwrap_or_else(|| collection.clone());

        let ctx = CompileCtx {
            primary_collection: &collection,
            primary_alias: &alias,
            join_aliases: Vec::new(),
            field_types: self.field_types,
        };

        let (filter, has_where) = if self.eat_kw("WHERE") {
            let expr = self.parse_expr()?;
            (compile_filter(&ctx, &expr)?, true)
        } else {
            (Document::new(), false)
        };

        Ok(MongoCommand::Delete {
            collection,
            filter,
            has_where,
        })
    }

    // ---- Expression precedence climbing ----

    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        while self.eat_kw("OR") {
            let right = self.parse_and()?;
            left = Expr::Binary {
                op: BinOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_not()?;
        while self.eat_kw("AND") {
            let right = self.parse_not()?;
            left = Expr::Binary {
                op: BinOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr> {
        if self.eat_kw("NOT") {
            let inner = self.parse_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr> {
        let left = self.parse_additive()?;

        // IS [NOT] NULL
        if self.eat_kw("IS") {
            let negated = self.eat_kw("NOT");
            self.expect_kw("NULL")?;
            return Ok(Expr::IsNull {
                expr: Box::new(left),
                negated,
            });
        }

        // [NOT] IN / BETWEEN / LIKE
        let negated = self.eat_kw("NOT");
        if self.eat_kw("IN") {
            self.expect_token(&Token::LParen)?;
            let mut list = Vec::new();
            loop {
                list.push(self.parse_expr()?);
                if matches!(self.peek(), Some(Token::Comma)) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            self.expect_token(&Token::RParen)?;
            return Ok(Expr::InList {
                expr: Box::new(left),
                list,
                negated,
            });
        }
        if self.eat_kw("BETWEEN") {
            let low = self.parse_additive()?;
            self.expect_kw("AND")?;
            let high = self.parse_additive()?;
            return Ok(Expr::Between {
                expr: Box::new(left),
                low: Box::new(low),
                high: Box::new(high),
                negated,
            });
        }
        if self.eat_kw("LIKE") {
            let pattern = match self.next() {
                Some(Token::StringLit(s)) => s,
                Some(other) => bail!("Expected string pattern after LIKE, got '{}'.", other.display()),
                None => bail!("Expected string pattern after LIKE."),
            };
            return Ok(Expr::Like {
                expr: Box::new(left),
                pattern,
                negated,
            });
        }
        if negated {
            bail!(
                "Expected IN, BETWEEN, or LIKE after NOT, got '{}'.",
                self.peek_display()
            );
        }

        let op = match self.peek() {
            Some(Token::Eq) => Some(BinOp::Eq),
            Some(Token::Neq) => Some(BinOp::Neq),
            Some(Token::Lt) => Some(BinOp::Lt),
            Some(Token::Lte) => Some(BinOp::Lte),
            Some(Token::Gt) => Some(BinOp::Gt),
            Some(Token::Gte) => Some(BinOp::Gte),
            _ => None,
        };
        if let Some(op) = op {
            self.pos += 1;
            let right = self.parse_additive()?;
            return Ok(Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            });
        }

        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinOp::Add,
                Some(Token::Minus) => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_multiplicative()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinOp::Mul,
                Some(Token::Slash) => BinOp::Div,
                Some(Token::Percent) => BinOp::Mod,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        if matches!(self.peek(), Some(Token::Minus)) {
            self.pos += 1;
            let inner = self.parse_unary()?;
            match inner {
                Expr::Literal(Bson::Int32(n)) => return Ok(Expr::Literal(Bson::Int64(-(n as i64)))),
                Expr::Literal(Bson::Int64(n)) => return Ok(Expr::Literal(Bson::Int64(-n))),
                Expr::Literal(Bson::Double(f)) => return Ok(Expr::Literal(Bson::Double(-f))),
                other => return Ok(Expr::UnaryNeg(Box::new(other))),
            }
        }
        if matches!(self.peek(), Some(Token::Plus)) {
            self.pos += 1;
            return self.parse_unary();
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let Some(tok) = self.peek().cloned() else {
            bail!("Unexpected end of SQL expression.");
        };

        match tok {
            Token::LParen => {
                self.pos += 1;
                let expr = self.parse_expr()?;
                self.expect_token(&Token::RParen)?;
                Ok(expr)
            }
            Token::StringLit(s) => {
                self.pos += 1;
                Ok(Expr::Literal(Bson::String(s)))
            }
            Token::IntLit(n) => {
                self.pos += 1;
                if let Ok(n32) = i32::try_from(n) {
                    Ok(Expr::Literal(Bson::Int32(n32)))
                } else {
                    Ok(Expr::Literal(Bson::Int64(n)))
                }
            }
            Token::FloatLit(f) => {
                self.pos += 1;
                Ok(Expr::Literal(Bson::Double(f)))
            }
            Token::Word(ref w) if w.eq_ignore_ascii_case("NULL") => {
                self.pos += 1;
                Ok(Expr::Literal(Bson::Null))
            }
            Token::Word(ref w) if w.eq_ignore_ascii_case("TRUE") => {
                self.pos += 1;
                Ok(Expr::Literal(Bson::Boolean(true)))
            }
            Token::Word(ref w) if w.eq_ignore_ascii_case("FALSE") => {
                self.pos += 1;
                Ok(Expr::Literal(Bson::Boolean(false)))
            }
            Token::Word(ref w) if w.eq_ignore_ascii_case("CASE") => {
                self.pos += 1;
                let mut branches = Vec::new();
                while self.eat_kw("WHEN") {
                    let cond = self.parse_expr()?;
                    self.expect_kw("THEN")?;
                    let val = self.parse_expr()?;
                    branches.push((cond, val));
                }
                let else_expr = if self.eat_kw("ELSE") {
                    Some(Box::new(self.parse_expr()?))
                } else {
                    None
                };
                self.expect_kw("END")?;
                Ok(Expr::Case {
                    branches,
                    else_expr,
                })
            }
            Token::Word(ref w)
                if (w.eq_ignore_ascii_case("DATE")
                    || w.eq_ignore_ascii_case("TIMESTAMP")
                    || w.eq_ignore_ascii_case("DATETIME"))
                    && matches!(self.peek_at(1), Some(Token::StringLit(_))) =>
            {
                self.pos += 1;
                let Some(Token::StringLit(s)) = self.next() else {
                    unreachable!()
                };
                let dt = parse_date_string(&s)?;
                Ok(Expr::Literal(Bson::DateTime(dt)))
            }
            Token::Word(ref w) if matches!(self.peek_at(1), Some(Token::LParen)) => {
                let name = w.clone();
                self.pos += 2; // name + '('
                let mut distinct = false;
                let mut star = false;
                let mut args = Vec::new();
                if !matches!(self.peek(), Some(Token::RParen)) {
                    if matches!(self.peek(), Some(Token::Star)) {
                        self.pos += 1;
                        star = true;
                    } else {
                        distinct = self.eat_kw("DISTINCT");
                        loop {
                            args.push(self.parse_expr()?);
                            if matches!(self.peek(), Some(Token::Comma)) {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                    }
                }
                self.expect_token(&Token::RParen)?;

                // Evaluate BSON constructor helpers eagerly if all arguments are constant.
                if is_bson_constructor(&name) && !star && !distinct {
                    let mut const_args = Vec::with_capacity(args.len());
                    for arg in &args {
                        const_args.push(eval_constant_expr(arg)?);
                    }
                    let bson = eval_bson_constructor(&name, &const_args)?;
                    return Ok(Expr::Literal(bson));
                }

                Ok(Expr::Call {
                    name,
                    distinct,
                    star,
                    args,
                })
            }
            Token::Word(_) | Token::QuotedIdent(_) => {
                let path = self.parse_ident_path()?;
                Ok(Expr::Column(path))
            }
            other => bail!("Unexpected token '{}' in SQL expression.", other.display()),
        }
    }
}

fn is_bson_constructor(name: &str) -> bool {
    name.eq_ignore_ascii_case("ObjectId")
        || name.eq_ignore_ascii_case("ISODate")
        || name.eq_ignore_ascii_case("Date")
        || name.eq_ignore_ascii_case("DateTime")
        || name.eq_ignore_ascii_case("UUID")
        || name.eq_ignore_ascii_case("GUID")
        || name.eq_ignore_ascii_case("NumberInt")
        || name.eq_ignore_ascii_case("NumberLong")
        || name.eq_ignore_ascii_case("NumberDecimal")
        || name.eq_ignore_ascii_case("Timestamp")
}

// ============================================================================
// Compilation helpers
// ============================================================================

struct CompileCtx<'a> {
    primary_collection: &'a str,
    primary_alias: &'a str,
    join_aliases: Vec<String>,
    field_types: &'a HashMap<String, String>,
}

impl<'a> CompileCtx<'a> {
    fn resolve_field(&self, parts: &[String]) -> String {
        if parts.len() >= 2 {
            let first = &parts[0];
            if first.eq_ignore_ascii_case(self.primary_alias)
                || first.eq_ignore_ascii_case(self.primary_collection)
            {
                return parts[1..].join(".");
            }
            if let Some(j_alias) = self
                .join_aliases
                .iter()
                .find(|a| a.eq_ignore_ascii_case(first))
            {
                return format!("{j_alias}.{}", parts[1..].join("."));
            }
        }
        parts.join(".")
    }
}

fn resolve_join_fields(
    ctx: &CompileCtx<'_>,
    join: &JoinClause,
    left_path: &[String],
    right_path: &[String],
) -> (String, String) {
    let left_is_joined = left_path.len() >= 2
        && (left_path[0].eq_ignore_ascii_case(&join.alias)
            || left_path[0].eq_ignore_ascii_case(&join.collection));
    let right_is_joined = right_path.len() >= 2
        && (right_path[0].eq_ignore_ascii_case(&join.alias)
            || right_path[0].eq_ignore_ascii_case(&join.collection));

    if left_is_joined && !right_is_joined {
        (ctx.resolve_field(right_path), left_path[1..].join("."))
    } else if right_is_joined {
        (ctx.resolve_field(left_path), right_path[1..].join("."))
    } else {
        (ctx.resolve_field(left_path), right_path.join("."))
    }
}

fn coerce_value_for_field(
    field_types: &HashMap<String, String>,
    field: &str,
    value: Bson,
) -> Bson {
    let Bson::String(ref s) = value else {
        return value;
    };
    let expected = field_types
        .get(field)
        .or_else(|| {
            field_types
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(field))
                .map(|(_, v)| v)
        })
        .map(String::as_str);

    match expected {
        Some("uuid") => parse_uuid_bson(s).unwrap_or(value),
        Some("objectId") => parse_object_id_bson(s).unwrap_or(value),
        Some("date") => parse_date_string(s).map(Bson::DateTime).unwrap_or(value),
        _ => value,
    }
}

fn insert_nested_field(doc: &mut Document, dotted: &str, val: Bson) {
    if !dotted.contains('.') {
        doc.insert(dotted.to_string(), val);
        return;
    }
    let parts: Vec<&str> = dotted.split('.').collect();
    let mut current = doc;
    for part in &parts[..parts.len() - 1] {
        let entry = current
            .entry((*part).to_string())
            .or_insert_with(|| Bson::Document(Document::new()));
        if !matches!(entry, Bson::Document(_)) {
            *entry = Bson::Document(Document::new());
        }
        let Bson::Document(sub) = entry else {
            unreachable!()
        };
        current = sub;
    }
    current.insert(parts[parts.len() - 1].to_string(), val);
}

fn eval_constant_expr(expr: &Expr) -> Result<Bson> {
    match expr {
        Expr::Literal(b) => Ok(b.clone()),
        Expr::UnaryNeg(inner) => match eval_constant_expr(inner)? {
            Bson::Int32(n) => Ok(Bson::Int64(-(n as i64))),
            Bson::Int64(n) => Ok(Bson::Int64(-n)),
            Bson::Double(f) => Ok(Bson::Double(-f)),
            other => bail!("Cannot negate non-numeric value {other}."),
        },
        Expr::Call {
            name,
            args,
            star: false,
            distinct: false,
        } => {
            if is_now_function(name) && args.is_empty() {
                return Ok(Bson::DateTime(BsonDateTime::now()));
            }
            let mut eval_args = Vec::with_capacity(args.len());
            for a in args {
                eval_args.push(eval_constant_expr(a)?);
            }
            eval_bson_constructor(name, &eval_args)
        }
        _ => bail!("Expected a constant value or BSON constructor in this position."),
    }
}

fn is_now_function(name: &str) -> bool {
    name.eq_ignore_ascii_case("NOW")
        || name.eq_ignore_ascii_case("GETDATE")
        || name.eq_ignore_ascii_case("GETUTCDATE")
        || name.eq_ignore_ascii_case("SYSDATETIME")
        || name.eq_ignore_ascii_case("SYSDATETIMEOFFSET")
        || name.eq_ignore_ascii_case("CURRENT_TIMESTAMP")
}

fn compile_filter(ctx: &CompileCtx<'_>, expr: &Expr) -> Result<Document> {
    match expr {
        Expr::Binary {
            op: BinOp::And,
            left,
            right,
        } => {
            let l = compile_filter(ctx, left)?;
            let r = compile_filter(ctx, right)?;
            Ok(merge_and_filters(l, r))
        }
        Expr::Binary {
            op: BinOp::Or,
            left,
            right,
        } => {
            let l = compile_filter(ctx, left)?;
            let r = compile_filter(ctx, right)?;
            let mut list = Vec::new();
            flatten_or(l, &mut list);
            flatten_or(r, &mut list);
            Ok(doc! { "$or": list })
        }
        Expr::Not(inner) => {
            // Compile `$nor: [inner]` which works cleanly for any sub-filter in MongoDB.
            let sub = compile_filter(ctx, inner)?;
            Ok(doc! { "$nor": [sub] })
        }
        Expr::IsNull { expr, negated } => {
            if let Expr::Column(parts) = expr.as_ref() {
                let field = ctx.resolve_field(parts);
                if *negated {
                    Ok(doc! { field: { "$ne": Bson::Null } })
                } else {
                    Ok(doc! { field: Bson::Null })
                }
            } else {
                let agg = compile_agg_expr(ctx, expr)?;
                let op = if *negated { "$ne" } else { "$eq" };
                Ok(doc! { "$expr": { op: [agg, Bson::Null] } })
            }
        }
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            if let Expr::Column(parts) = expr.as_ref() {
                let field = ctx.resolve_field(parts);
                let mut vals = Vec::with_capacity(list.len());
                for item in list {
                    let v = eval_constant_expr(item)?;
                    vals.push(coerce_value_for_field(ctx.field_types, &field, v));
                }
                let op = if *negated { "$nin" } else { "$in" };
                Ok(doc! { field: { op: vals } })
            } else {
                let agg = compile_agg_expr(ctx, expr)?;
                let mut vals = Vec::with_capacity(list.len());
                for item in list {
                    vals.push(compile_agg_expr(ctx, item)?);
                }
                let in_expr = Bson::Document(doc! { "$in": [agg, vals] });
                if *negated {
                    Ok(doc! { "$expr": { "$not": [in_expr] } })
                } else {
                    Ok(doc! { "$expr": in_expr })
                }
            }
        }
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => {
            if let Expr::Column(parts) = expr.as_ref() {
                let field = ctx.resolve_field(parts);
                let lo = coerce_value_for_field(ctx.field_types, &field, eval_constant_expr(low)?);
                let hi = coerce_value_for_field(ctx.field_types, &field, eval_constant_expr(high)?);
                if *negated {
                    Ok(doc! {
                        "$or": [
                            { &field: { "$lt": lo } },
                            { &field: { "$gt": hi } },
                        ]
                    })
                } else {
                    Ok(doc! { field: { "$gte": lo, "$lte": hi } })
                }
            } else {
                let a = compile_agg_expr(ctx, expr)?;
                let lo = compile_agg_expr(ctx, low)?;
                let hi = compile_agg_expr(ctx, high)?;
                let cond = Bson::Document(doc! {
                    "$and": [
                        { "$gte": [a.clone(), lo] },
                        { "$lte": [a, hi] },
                    ]
                });
                if *negated {
                    Ok(doc! { "$expr": { "$not": [cond] } })
                } else {
                    Ok(doc! { "$expr": cond })
                }
            }
        }
        Expr::Like {
            expr,
            pattern,
            negated,
        } => {
            let regex = sql_like_to_regex(pattern);
            if let Expr::Column(parts) = expr.as_ref() {
                let field = ctx.resolve_field(parts);
                if *negated {
                    Ok(doc! {
                        field: {
                            "$not": Bson::RegularExpression(Regex {
                                pattern: regex,
                                options: "i".to_string(),
                            })
                        }
                    })
                } else {
                    Ok(doc! {
                        field: {
                            "$regex": regex,
                            "$options": "i",
                        }
                    })
                }
            } else {
                let agg = compile_agg_expr(ctx, expr)?;
                let rm = Bson::Document(doc! {
                    "$regexMatch": {
                        "input": agg,
                        "regex": regex,
                        "options": "i",
                    }
                });
                if *negated {
                    Ok(doc! { "$expr": { "$not": [rm] } })
                } else {
                    Ok(doc! { "$expr": rm })
                }
            }
        }
        Expr::Binary { op, left, right }
            if matches!(
                op,
                BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Lte | BinOp::Gt | BinOp::Gte
            ) =>
        {
            // Fast path: `field <op> constant` or `constant <op> field` -> direct index-friendly MongoDB filter!
            if let (Expr::Column(parts), Ok(val)) = (left.as_ref(), eval_constant_expr(right)) {
                let field = ctx.resolve_field(parts);
                let coerced = coerce_value_for_field(ctx.field_types, &field, val);
                return Ok(build_field_cmp(&field, *op, coerced));
            }
            if let (Ok(val), Expr::Column(parts)) = (eval_constant_expr(left), right.as_ref()) {
                let field = ctx.resolve_field(parts);
                let coerced = coerce_value_for_field(ctx.field_types, &field, val);
                return Ok(build_field_cmp(&field, flip_cmp(*op), coerced));
            }
            // General expression comparison using `$expr`.
            let l_agg = compile_agg_expr(ctx, left)?;
            let r_agg = compile_agg_expr(ctx, right)?;
            let op_name = cmp_op_name(*op);
            Ok(doc! { "$expr": { op_name: [l_agg, r_agg] } })
        }
        other => {
            let agg = compile_agg_expr(ctx, other)?;
            Ok(doc! { "$expr": agg })
        }
    }
}

fn build_field_cmp(field: &str, op: BinOp, val: Bson) -> Document {
    match op {
        BinOp::Eq => doc! { field: val },
        BinOp::Neq => doc! { field: { "$ne": val } },
        BinOp::Lt => doc! { field: { "$lt": val } },
        BinOp::Lte => doc! { field: { "$lte": val } },
        BinOp::Gt => doc! { field: { "$gt": val } },
        BinOp::Gte => doc! { field: { "$gte": val } },
        _ => unreachable!(),
    }
}

fn flip_cmp(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Gt,
        BinOp::Lte => BinOp::Gte,
        BinOp::Gt => BinOp::Lt,
        BinOp::Gte => BinOp::Lte,
        other => other,
    }
}

fn cmp_op_name(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "$eq",
        BinOp::Neq => "$ne",
        BinOp::Lt => "$lt",
        BinOp::Lte => "$lte",
        BinOp::Gt => "$gt",
        BinOp::Gte => "$gte",
        _ => unreachable!(),
    }
}

fn merge_and_filters(mut a: Document, b: Document) -> Document {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    // Check for overlapping keys; if none overlap, merge directly into one clean document.
    let has_overlap = b.keys().any(|k| a.contains_key(k));
    if !has_overlap {
        for (k, v) in b {
            a.insert(k, v);
        }
        a
    } else {
        let mut items = Vec::new();
        flatten_and(a, &mut items);
        flatten_and(b, &mut items);
        doc! { "$and": items }
    }
}

fn flatten_and(d: Document, out: &mut Vec<Bson>) {
    if d.len() == 1 {
        if let Some(Bson::Array(arr)) = d.get("$and") {
            out.extend(arr.clone());
            return;
        }
    }
    out.push(Bson::Document(d));
}

fn flatten_or(d: Document, out: &mut Vec<Bson>) {
    if d.len() == 1 {
        if let Some(Bson::Array(arr)) = d.get("$or") {
            out.extend(arr.clone());
            return;
        }
    }
    out.push(Bson::Document(d));
}

fn sql_like_to_regex(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 4);
    out.push('^');
    for ch in pattern.chars() {
        match ch {
            '%' => out.push_str(".*"),
            '_' => out.push('.'),
            '.' | '+' | '*' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            other => out.push(other),
        }
    }
    out.push('$');
    out
}

fn compile_agg_expr(ctx: &CompileCtx<'_>, expr: &Expr) -> Result<Bson> {
    match expr {
        Expr::Literal(b) => match b {
            Bson::String(s) if s.starts_with('$') => Ok(Bson::Document(doc! { "$literal": s })),
            other => Ok(other.clone()),
        },
        Expr::Column(parts) => Ok(Bson::String(format!("${}", ctx.resolve_field(parts)))),
        Expr::UnaryNeg(inner) => {
            let v = compile_agg_expr(ctx, inner)?;
            Ok(Bson::Document(doc! { "$multiply": [-1, v] }))
        }
        Expr::Not(inner) => {
            let v = compile_agg_expr(ctx, inner)?;
            Ok(Bson::Document(doc! { "$not": [v] }))
        }
        Expr::Binary { op, left, right } => {
            let l = compile_agg_expr(ctx, left)?;
            let r = compile_agg_expr(ctx, right)?;
            let mongo_op = match op {
                BinOp::And => "$and",
                BinOp::Or => "$or",
                BinOp::Eq => "$eq",
                BinOp::Neq => "$ne",
                BinOp::Lt => "$lt",
                BinOp::Lte => "$lte",
                BinOp::Gt => "$gt",
                BinOp::Gte => "$gte",
                BinOp::Add => "$add",
                BinOp::Sub => "$subtract",
                BinOp::Mul => "$multiply",
                BinOp::Div => "$divide",
                BinOp::Mod => "$mod",
            };
            Ok(Bson::Document(doc! { mongo_op: [l, r] }))
        }
        Expr::IsNull { expr, negated } => {
            let v = compile_agg_expr(ctx, expr)?;
            let op = if *negated { "$ne" } else { "$eq" };
            Ok(Bson::Document(doc! { op: [v, Bson::Null] }))
        }
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            let v = compile_agg_expr(ctx, expr)?;
            let mut vals = Vec::with_capacity(list.len());
            for item in list {
                vals.push(compile_agg_expr(ctx, item)?);
            }
            let in_doc = Bson::Document(doc! { "$in": [v, vals] });
            if *negated {
                Ok(Bson::Document(doc! { "$not": [in_doc] }))
            } else {
                Ok(in_doc)
            }
        }
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => {
            let v = compile_agg_expr(ctx, expr)?;
            let lo = compile_agg_expr(ctx, low)?;
            let hi = compile_agg_expr(ctx, high)?;
            let cond = Bson::Document(doc! {
                "$and": [
                    { "$gte": [v.clone(), lo] },
                    { "$lte": [v, hi] },
                ]
            });
            if *negated {
                Ok(Bson::Document(doc! { "$not": [cond] }))
            } else {
                Ok(cond)
            }
        }
        Expr::Like {
            expr,
            pattern,
            negated,
        } => {
            let v = compile_agg_expr(ctx, expr)?;
            let rm = Bson::Document(doc! {
                "$regexMatch": {
                    "input": v,
                    "regex": sql_like_to_regex(pattern),
                    "options": "i",
                }
            });
            if *negated {
                Ok(Bson::Document(doc! { "$not": [rm] }))
            } else {
                Ok(rm)
            }
        }
        Expr::Case {
            branches,
            else_expr,
        } => {
            let mut acc = match else_expr {
                Some(e) => compile_agg_expr(ctx, e)?,
                None => Bson::Null,
            };
            for (cond, val) in branches.iter().rev() {
                let c = compile_agg_expr(ctx, cond)?;
                let t = compile_agg_expr(ctx, val)?;
                acc = Bson::Document(doc! { "$cond": [c, t, acc] });
            }
            Ok(acc)
        }
        Expr::Call { name, args, .. } => compile_scalar_function(ctx, name, args),
    }
}

fn compile_scalar_function(ctx: &CompileCtx<'_>, name: &str, args: &[Expr]) -> Result<Bson> {
    let upper = name.to_ascii_uppercase();
    if is_now_function(&upper) {
        return Ok(Bson::String("$$NOW".to_string()));
    }
    let mut compiled = Vec::with_capacity(args.len());
    for a in args {
        compiled.push(compile_agg_expr(ctx, a)?);
    }

    match upper.as_str() {
        "UPPER" => single_arg_op(&upper, "$toUpper", compiled),
        "LOWER" => single_arg_op(&upper, "$toLower", compiled),
        "LEN" | "LENGTH" => single_arg_op(&upper, "$strLenCP", compiled),
        "ABS" => single_arg_op(&upper, "$abs", compiled),
        "CEILING" | "CEIL" => single_arg_op(&upper, "$ceil", compiled),
        "FLOOR" => single_arg_op(&upper, "$floor", compiled),
        "SQRT" => single_arg_op(&upper, "$sqrt", compiled),
        "YEAR" => single_arg_op(&upper, "$year", compiled),
        "MONTH" => single_arg_op(&upper, "$month", compiled),
        "DAY" => single_arg_op(&upper, "$dayOfMonth", compiled),
        "HOUR" => single_arg_op(&upper, "$hour", compiled),
        "MINUTE" => single_arg_op(&upper, "$minute", compiled),
        "SECOND" => single_arg_op(&upper, "$second", compiled),
        "TRIM" => {
            if compiled.len() != 1 {
                bail!("TRIM() expects 1 argument.");
            }
            Ok(Bson::Document(doc! { "$trim": { "input": compiled.remove(0) } }))
        }
        "LTRIM" => {
            if compiled.len() != 1 {
                bail!("LTRIM() expects 1 argument.");
            }
            Ok(Bson::Document(doc! { "$ltrim": { "input": compiled.remove(0) } }))
        }
        "RTRIM" => {
            if compiled.len() != 1 {
                bail!("RTRIM() expects 1 argument.");
            }
            Ok(Bson::Document(doc! { "$rtrim": { "input": compiled.remove(0) } }))
        }
        "ROUND" => {
            if compiled.is_empty() || compiled.len() > 2 {
                bail!("ROUND() expects 1 or 2 arguments.");
            }
            Ok(Bson::Document(doc! { "$round": compiled }))
        }
        "POWER" | "POW" => {
            if compiled.len() != 2 {
                bail!("POWER() expects 2 arguments.");
            }
            Ok(Bson::Document(doc! { "$pow": compiled }))
        }
        "CONCAT" => Ok(Bson::Document(doc! { "$concat": compiled })),
        "COALESCE" | "ISNULL" | "IFNULL" => {
            if compiled.len() < 2 {
                bail!("{upper}() expects at least 2 arguments.");
            }
            let mut acc = compiled.pop().unwrap();
            while let Some(prev) = compiled.pop() {
                acc = Bson::Document(doc! { "$ifNull": [prev, acc] });
            }
            Ok(acc)
        }
        "SUBSTRING" | "SUBSTR" => {
            if compiled.len() != 3 {
                bail!("{upper}(str, start, len) expects 3 arguments.");
            }
            let s = compiled.remove(0);
            let start = compiled.remove(0);
            let len = compiled.remove(0);
            // SQL SUBSTRING is 1-based; MongoDB $substrCP is 0-based.
            let zero_based = Bson::Document(doc! { "$subtract": [start, 1] });
            Ok(Bson::Document(doc! { "$substrCP": [s, zero_based, len] }))
        }
        _ => bail!("Unsupported SQL function '{name}' in MongoDB query."),
    }
}

fn single_arg_op(fn_name: &str, mongo_op: &str, mut args: Vec<Bson>) -> Result<Bson> {
    if args.len() != 1 {
        bail!("{fn_name}() expects 1 argument.");
    }
    Ok(Bson::Document(doc! { mongo_op: args.remove(0) }))
}

fn is_aggregate_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("COUNT")
        || name.eq_ignore_ascii_case("SUM")
        || name.eq_ignore_ascii_case("AVG")
        || name.eq_ignore_ascii_case("MIN")
        || name.eq_ignore_ascii_case("MAX")
}

fn expr_contains_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Call { name, args, .. } => {
            is_aggregate_name(name) || args.iter().any(expr_contains_aggregate)
        }
        Expr::UnaryNeg(e) | Expr::Not(e) | Expr::IsNull { expr: e, .. } => {
            expr_contains_aggregate(e)
        }
        Expr::Binary { left, right, .. } => {
            expr_contains_aggregate(left) || expr_contains_aggregate(right)
        }
        Expr::InList { expr, list, .. } => {
            expr_contains_aggregate(expr) || list.iter().any(expr_contains_aggregate)
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_aggregate(expr)
                || expr_contains_aggregate(low)
                || expr_contains_aggregate(high)
        }
        Expr::Like { expr, .. } => expr_contains_aggregate(expr),
        Expr::Case {
            branches,
            else_expr,
        } => {
            branches
                .iter()
                .any(|(c, v)| expr_contains_aggregate(c) || expr_contains_aggregate(v))
                || else_expr
                    .as_deref()
                    .is_some_and(expr_contains_aggregate)
        }
        Expr::Literal(_) | Expr::Column(_) => false,
    }
}

/// Returns `(accumulator_doc, is_distinct_count)` if `expr` is an aggregate call.
fn try_compile_accumulator(ctx: &CompileCtx<'_>, expr: &Expr) -> Result<Option<(Document, bool)>> {
    let Expr::Call {
        name,
        distinct,
        star,
        args,
    } = expr
    else {
        return Ok(None);
    };
    let upper = name.to_ascii_uppercase();
    match upper.as_str() {
        "COUNT" => {
            if *star {
                Ok(Some((doc! { "$sum": 1 }, false)))
            } else if args.len() == 1 {
                let inner = compile_agg_expr(ctx, &args[0])?;
                if *distinct {
                    Ok(Some((doc! { "$addToSet": inner }, true)))
                } else {
                    Ok(Some((
                        doc! {
                            "$sum": {
                                "$cond": [{ "$ne": [inner, Bson::Null] }, 1, 0]
                            }
                        },
                        false,
                    )))
                }
            } else {
                bail!("COUNT() expects * or 1 argument.");
            }
        }
        "SUM" => {
            if args.len() != 1 {
                bail!("SUM() expects 1 argument.");
            }
            let inner = compile_agg_expr(ctx, &args[0])?;
            Ok(Some((doc! { "$sum": inner }, false)))
        }
        "AVG" => {
            if args.len() != 1 {
                bail!("AVG() expects 1 argument.");
            }
            let inner = compile_agg_expr(ctx, &args[0])?;
            Ok(Some((doc! { "$avg": inner }, false)))
        }
        "MIN" => {
            if args.len() != 1 {
                bail!("MIN() expects 1 argument.");
            }
            let inner = compile_agg_expr(ctx, &args[0])?;
            Ok(Some((doc! { "$min": inner }, false)))
        }
        "MAX" => {
            if args.len() != 1 {
                bail!("MAX() expects 1 argument.");
            }
            let inner = compile_agg_expr(ctx, &args[0])?;
            Ok(Some((doc! { "$max": inner }, false)))
        }
        _ => Ok(None),
    }
}

fn compile_agg_expr_with_group(
    ctx: &CompileCtx<'_>,
    expr: &Expr,
    group_keys: &[(String, Expr)],
) -> Result<Bson> {
    if let Some((key, _)) = group_keys
        .iter()
        .find(|(_, ge)| exprs_structurally_equal(ge, expr))
    {
        return Ok(Bson::String(format!("$_id.{key}")));
    }
    compile_agg_expr(ctx, expr)
}

fn rewrite_having_expr(expr: &Expr, select_items: &[SelectItem]) -> Expr {
    // If HAVING references an aggregate expression that appeared in the SELECT list, rewrite it to its output column name.
    for (idx, item) in select_items.iter().enumerate() {
        if let SelectItem::Expr {
            expr: sel_expr,
            alias,
        } = item
        {
            if exprs_structurally_equal(expr, sel_expr) {
                let col = alias
                    .clone()
                    .unwrap_or_else(|| default_expr_name(sel_expr, idx));
                return Expr::Column(vec![col]);
            }
        }
    }
    match expr {
        Expr::Binary { op, left, right } => Expr::Binary {
            op: *op,
            left: Box::new(rewrite_having_expr(left, select_items)),
            right: Box::new(rewrite_having_expr(right, select_items)),
        },
        Expr::Not(inner) => Expr::Not(Box::new(rewrite_having_expr(inner, select_items))),
        Expr::UnaryNeg(inner) => Expr::UnaryNeg(Box::new(rewrite_having_expr(inner, select_items))),
        other => other.clone(),
    }
}

fn default_expr_name(expr: &Expr, index: usize) -> String {
    match expr {
        Expr::Column(parts) => parts.last().cloned().unwrap_or_else(|| format!("col_{index}")),
        Expr::Call { name, star, .. } => {
            if *star {
                format!("{}_all", name.to_ascii_lowercase())
            } else {
                format!("{}_{index}", name.to_ascii_lowercase())
            }
        }
        _ => format!("col_{index}"),
    }
}

fn exprs_structurally_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Column(pa), Expr::Column(pb)) => {
            pa.len() == pb.len()
                && pa
                    .iter()
                    .zip(pb.iter())
                    .all(|(x, y)| x.eq_ignore_ascii_case(y))
        }
        (
            Expr::Call {
                name: na,
                distinct: da,
                star: sa,
                args: aa,
            },
            Expr::Call {
                name: nb,
                distinct: db,
                star: sb,
                args: ab,
            },
        ) => {
            na.eq_ignore_ascii_case(nb)
                && da == db
                && sa == sb
                && aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(x, y)| exprs_structurally_equal(x, y))
        }
        (Expr::Literal(la), Expr::Literal(lb)) => la == lb,
        _ => false,
    }
}

/// Returns true if any pipeline stage uses a write stage (`$out` or `$merge`).
pub fn pipeline_has_write_stage(pipeline: &[Document]) -> bool {
    let mut stack: Vec<&Document> = pipeline.iter().collect();
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    while let Some(doc) = stack.pop() {
        let ptr = doc as *const Document as usize;
        if !seen.insert(ptr) {
            continue;
        }
        for (k, v) in doc {
            if k == "$out" || k == "$merge" {
                return true;
            }
            collect_nested_docs(v, &mut stack);
        }
    }
    false
}

fn collect_nested_docs<'a>(v: &'a Bson, stack: &mut Vec<&'a Document>) {
    match v {
        Bson::Document(d) => stack.push(d),
        Bson::Array(arr) => {
            for item in arr {
                collect_nested_docs(item, stack);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_select_where_order_limit_and_coercion() {
        let mut types = HashMap::new();
        types.insert("_id".to_string(), "objectId".to_string());
        types.insert("tenant_id".to_string(), "uuid".to_string());

        let cmd = TSqlMongoCompiler::compile_with_field_types(
            "SELECT TOP 10 [name], [email] FROM [users] WHERE [_id] = '507f1f77bcf86cd799439011' AND [age] >= 21 ORDER BY [name] ASC",
            &types,
        )
        .unwrap();

        let MongoCommand::Select {
            collection,
            pipeline,
            explicit_columns,
            limit,
        } = cmd
        else {
            panic!("expected Select");
        };
        assert_eq!(collection, "users");
        assert_eq!(explicit_columns, vec!["name", "email"]);
        assert_eq!(limit, Some(10));
        // $match should coerce _id to ObjectId
        let match_stage = pipeline[0].get_document("$match").unwrap();
        assert!(matches!(match_stage.get("_id"), Some(Bson::ObjectId(_))));
    }

    #[test]
    fn compiles_group_by_having_and_count_distinct() {
        let cmd = TSqlMongoCompiler::compile(
            "SELECT [city], COUNT(*) AS [n], COUNT(DISTINCT [email]) AS [uniq] FROM [users] GROUP BY [city] HAVING [n] > 5 ORDER BY [n] DESC LIMIT 20",
        )
        .unwrap();

        let MongoCommand::Select {
            collection,
            pipeline,
            explicit_columns,
            limit,
        } = cmd
        else {
            panic!("expected Select");
        };
        assert_eq!(collection, "users");
        assert_eq!(explicit_columns, vec!["city", "n", "uniq"]);
        assert_eq!(limit, Some(20));
        assert!(pipeline.iter().any(|s| s.contains_key("$group")));
        assert!(pipeline.iter().any(|s| s.contains_key("$project")));
        assert!(pipeline.iter().any(|s| s.contains_key("$match")));
        assert!(pipeline.iter().any(|s| s.contains_key("$sort")));
    }

    #[test]
    fn compiles_join_insert_update_delete_and_use() {
        let join_cmd = TSqlMongoCompiler::compile(
            "SELECT o.[total], c.[name] FROM [orders] o LEFT JOIN [customers] c ON o.[customer_id] = c.[_id]",
        )
        .unwrap();
        assert!(matches!(join_cmd, MongoCommand::Select { .. }));
        if let MongoCommand::Select { pipeline, .. } = join_cmd {
            assert!(pipeline[0].contains_key("$lookup"));
            assert!(pipeline[1].contains_key("$unwind"));
        }

        let ins = TSqlMongoCompiler::compile(
            "INSERT INTO [users] ([name], [age]) VALUES ('Ada', 36), ('Grace', 85)",
        )
        .unwrap();
        match ins {
            MongoCommand::Insert {
                collection,
                documents,
            } => {
                assert_eq!(collection, "users");
                assert_eq!(documents.len(), 2);
                assert_eq!(documents[0].get_str("name").unwrap(), "Ada");
            }
            _ => panic!("expected Insert"),
        }

        let upd = TSqlMongoCompiler::compile(
            "UPDATE [users] SET [status] = 'active' WHERE [age] > 18",
        )
        .unwrap();
        assert!(matches!(upd, MongoCommand::Update { .. }));

        let del_blanket = TSqlMongoCompiler::compile("DELETE FROM [users]").unwrap();
        assert!(matches!(
            del_blanket,
            MongoCommand::Delete {
                has_where: false,
                ..
            }
        ));

        let use_cmd = TSqlMongoCompiler::compile("USE [shop]").unwrap();
        assert_eq!(
            use_cmd,
            MongoCommand::Use {
                database: "shop".to_string()
            }
        );
    }

    #[test]
    fn compiles_like_between_in_and_null_predicates() {
        let cmd = TSqlMongoCompiler::compile(
            "SELECT * FROM [orders] WHERE [code] LIKE 'ORD_%' AND [total] BETWEEN 10 AND 100 AND [status] IN ('open', 'paid') AND [deleted_at] IS NULL",
        )
        .unwrap();
        let MongoCommand::Select { pipeline, .. } = cmd else {
            panic!("expected Select");
        };
        let m = pipeline[0].get_document("$match").unwrap();
        assert!(m.contains_key("code"));
        assert!(m.contains_key("total"));
        assert!(m.contains_key("status"));
        assert_eq!(m.get("deleted_at"), Some(&Bson::Null));
    }
}
