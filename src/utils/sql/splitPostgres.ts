/**
 * PostgreSQL statement splitter (frontend).
 * Mirrors src-tauri/src/db/postgres/batch.rs: splits on `;` outside
 * '' strings (plus backslash escapes in E'...'), "quoted identifiers",
 * dollar-quoted bodies ($$...$$, $tag$...$tag$), -- and (nested) block
 * comments. Statements are returned trimmed, without the terminator — the
 * same shape as the T-SQL splitter.
 */

function isIdentChar(ch: string | undefined): boolean {
  return ch !== undefined && /[\p{L}\p{N}_$]/u.test(ch);
}

/** Length of a `$$` / `$tag$` opener at `i`, or 0 (e.g. a `$1` placeholder). */
function dollarTagLength(sql: string, i: number): number {
  const next = sql[i + 1];
  if (next === '$') return 2;
  if (next === undefined || !/[\p{L}_]/u.test(next)) return 0;
  for (let j = i + 2; j < sql.length; j++) {
    if (sql[j] === '$') return j - i + 1;
    if (!/[\p{L}\p{N}_]/u.test(sql[j])) return 0;
  }
  return 0;
}

/**
 * If a comment, string, quoted identifier or dollar-quoted body starts at
 * `i`, the index just past it (end of input when unterminated); else null.
 */
export function skipPostgresNonCode(sql: string, i: number): number | null {
  const ch = sql[i];
  const next = sql[i + 1];

  if (ch === '-' && next === '-') {
    const end = sql.indexOf('\n', i);
    return end === -1 ? sql.length : end;
  }

  if (ch === '/' && next === '*') {
    let depth = 0;
    let j = i;
    while (j < sql.length) {
      if (sql[j] === '/' && sql[j + 1] === '*') {
        depth++;
        j += 2;
      } else if (sql[j] === '*' && sql[j + 1] === '/') {
        depth--;
        j += 2;
        if (depth === 0) return j;
      } else {
        j++;
      }
    }
    return sql.length;
  }

  if (ch === "'" || ch === '"') {
    // E'...' escape strings honour backslash escapes
    const escapes = ch === "'" && /[Ee]/.test(sql[i - 1] ?? '') && !isIdentChar(sql[i - 2]);
    let j = i + 1;
    while (j < sql.length) {
      if (escapes && sql[j] === '\\') {
        j += 2;
        continue;
      }
      if (sql[j] === ch) {
        if (sql[j + 1] === ch) {
          j += 2;
          continue;
        }
        return j + 1;
      }
      j++;
    }
    return sql.length;
  }

  if (ch === '$' && !isIdentChar(sql[i - 1])) {
    const tagLength = dollarTagLength(sql, i);
    if (tagLength === 0) return null;
    const tag = sql.slice(i, i + tagLength);
    const close = sql.indexOf(tag, i + tagLength);
    return close === -1 ? sql.length : close + tagLength;
  }

  return null;
}

export function splitPostgresStatements(sql: string): string[] {
  const statements: string[] = [];
  let start = 0;
  let i = 0;

  const flush = (end: number) => {
    const text = sql.slice(start, end).trim();
    if (text && !isCommentOnly(text)) statements.push(text);
  };

  while (i < sql.length) {
    const skipped = skipPostgresNonCode(sql, i);
    if (skipped !== null) {
      i = skipped;
      continue;
    }
    if (sql[i] === ';') {
      flush(i);
      start = i + 1;
    }
    i++;
  }
  flush(sql.length);
  return statements;
}

/** Whitespace/comment-only fragments are not statements. */
function isCommentOnly(text: string): boolean {
  let i = 0;
  while (i < text.length) {
    if (/\s/.test(text[i])) {
      i++;
      continue;
    }
    if ((text[i] === '-' && text[i + 1] === '-') || (text[i] === '/' && text[i + 1] === '*')) {
      i = skipPostgresNonCode(text, i) ?? text.length;
      continue;
    }
    return false;
  }
  return true;
}
