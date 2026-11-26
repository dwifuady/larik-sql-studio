# ADR 0005: Cross-Dialect SQL Lexing, Autocomplete Ranking & Snippet Expansion

- **Status:** Accepted
- **Date:** 2026-11-26
- **Scope:** `crates/larik-data/src/lib.rs`, `crates/larik-data/src/sql_language.rs`

## Context

Larik SQL Studio shares a single SQL editor, statement splitter, snippet expander, and completion engine across T-SQL, PostgreSQL, MySQL, SQLite, and MongoDB's hybrid SQL/mongosh editor. Subtle lexing and ranking rules are essential so completions feel helpful rather than noisy or syntactically invalid.

## Decisions

### 1. Cross-Dialect SQL Lexing & Statement Splitting
- **PostgreSQL dollar-quoting:** `try_parse_dollar_quote_tag` matches `$$` or `$tag$` where `tag` starts with `[A-Za-z_]` (or non-ASCII) and continues with `[A-Za-z0-9_]` (or non-ASCII), explicitly rejecting numeric positional parameters like `$1` and `$2`.
- **Unified semicolon splitter:** `split_semicolon_statements` respects SQL quotes, comments, PostgreSQL dollar-quoted strings (`$$...$$` / `$tag$...$tag$`), `E'...'` escape strings, and opt-in `//` line comments for MongoDB shell/SQL scripts.

### 2. Snippet Template DSL & Expansion Contract
`expand_snippet` expands snippet markup into insertable text plus post-insert caret/selection UTF-8 byte offsets relative to the start of the inserted text:
- `${cursor}` records the final caret byte offset.
- `${paste}` appends the clipboard text.
- `${n:default}` / `${n}` (for non-empty ASCII digit `n`): the first occurrence records `default` and sets `selection` if still `None`; repeated occurrences of `n` reuse the remembered `default`.
- Unknown `${...}` tokens or unclosed `${` sequences are emitted verbatim.

### 3. SQL Autocomplete Suppression, Ranking & Quoting Rules
- **Trailing-whitespace suppression:** Auto-completion does not pop up uninvited on trailing whitespace after a completed expression unless the caret is immediately after a clause trigger (`FROM `, `JOIN `, `ON `, `USE `, `SELECT `, etc.), after a qualifier dot, or explicitly invoked with `Ctrl+Space`.
- **Two-tier `match_rank` priority:**
  - `match_rank = 0` (direct/prefix match: trigger prefix, identifier prefix, keyword prefix) universally outranks `match_rank = 1` (weak/substring match: snippet name substring, mid-identifier match).
  - Direct snippet trigger prefix matches get `match_rank = 0, kind_rank = 0` so they always lead ahead of keywords, tables, and columns.
  - Snippet name substring matches get `match_rank = 1` so direct keyword/table prefixes (e.g., `wher` -> `WHERE`) outrank snippet descriptions.
- **Alias ranking in column scope:** Table/alias qualifiers are offered inside column scope so typing an alias prefix can complete `[o]`, using `CompletionKind::Column` with `rank_sort = 10` so they remain completable without outranking direct column prefix matches.
- **Identifier quoting & replacement spans:**
  - `add_tables` always dialect-quotes schema and table identifiers, omitting the schema prefix from `insert_text` when the user already typed a schema qualifier (e.g., `FROM dbo.<caret>` or `FROM [dbo].<caret>`) so insertion never duplicates `dbo.[dbo].[Orders]`.
  - `word_start` steps back over a leading `[`, `"`, or `` ` `` so inserting a dialect-quoted identifier replaces cleanly without doubling the opening delimiter.
- **JOIN `ON` condition inference:** Suggestions prioritize declared foreign keys (both forward and reverse, including composite FKs) ahead of name-based heuristics (`customer_id -> customers.id`) and shared column names.
