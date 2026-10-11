import { getResultStatementLabel } from '../sql';

/**
 * mongo-shell / SQL statement splitting for the editor.
 *
 * Mirrors the backend splitter (`split_statements` in
 * src-tauri/src/db/mongodb/shell.rs) so run-at-cursor and CodeLens agree with
 * what the server executes:
 * - shell statements end at `;` or where a top-level `db` / `show` / `use` /
 *   `select` begins (not after `.`, so chains may continue on the next line)
 * - `show` / `use` end at end of line
 * - SQL statements (`SELECT ...`) end at `;`, or at a statement keyword that
 *   starts a line; `--` comments are allowed in them
 */

export interface MongoStatement {
  statement: string;
  /** Character offsets into the source text */
  start: number;
  end: number;
  /** 1-based Monaco positions (end is exclusive) */
  startLine: number;
  startColumn: number;
  endLine: number;
  endColumn: number;
}

type StatementKind = 'shell' | 'line' | 'sql';

function keywordKind(word: string): StatementKind | null {
  if (word === 'db') return 'shell';
  if (word === 'show' || word === 'use') return 'line';
  if (word.toLowerCase() === 'select') return 'sql';
  return null;
}

/** Whether a statement is SQL (`SELECT ...`) rather than shell syntax */
export function isSqlStatement(text: string): boolean {
  const stripped = text.replace(/^(\s|\/\/[^\n]*|--[^\n]*|\/\*[\s\S]*?\*\/)*/, '');
  return /^select\b/i.test(stripped);
}

function isIdentStart(c: string): boolean {
  return /[A-Za-z_$]/.test(c);
}

function isIdentChar(c: string): boolean {
  return /[A-Za-z0-9_$]/.test(c);
}

function positionAt(text: string, offset: number): { line: number; column: number } {
  let line = 1;
  let lastNewline = -1;
  for (let i = 0; i < offset && i < text.length; i++) {
    if (text[i] === '\n') {
      line++;
      lastNewline = i;
    }
  }
  return { line, column: offset - lastNewline };
}

/** Split editor text into top-level statements (shell or SQL) */
export function splitMongoStatements(text: string): MongoStatement[] {
  const ranges: Array<{ start: number; end: number }> = [];
  let i = 0;
  let depth = 0;
  let start = -1; // start of the current statement
  let kind: StatementKind = 'shell';
  let lastSignificantEnd = -1; // end offset of the last non-trivia character
  let lastSignificantChar = '';
  let lineHasToken = false;

  const finish = (end: number) => {
    if (start >= 0 && end > start) ranges.push({ start, end });
    start = -1;
  };

  while (i < text.length) {
    const c = text[i];
    const next = text[i + 1];

    if (c === '\n') {
      if (kind === 'line' && depth === 0) finish(lastSignificantEnd);
      lineHasToken = false;
      i++;
      continue;
    }
    if (/\s/.test(c)) {
      i++;
      continue;
    }

    // Comments (`--` only in SQL, where it can't be a decrement)
    const sqlComment = c === '-' && next === '-' && (kind === 'sql' || start < 0);
    if ((c === '/' && next === '/') || sqlComment) {
      while (i < text.length && text[i] !== '\n') i++;
      continue;
    }
    if (c === '/' && next === '*') {
      const close = text.indexOf('*/', i + 2);
      i = close === -1 ? text.length : close + 2;
      continue;
    }

    if (c === ';' && depth === 0) {
      finish(lastSignificantEnd);
      i++;
      lastSignificantChar = ';';
      lineHasToken = true;
      continue;
    }

    // Words: a statement keyword may begin a new statement
    if (isIdentStart(c) && (i === 0 || !isIdentChar(text[i - 1]))) {
      let j = i;
      while (j < text.length && isIdentChar(text[j])) j++;
      const word = text.slice(i, j);
      const wordKind = keywordKind(word);
      const startsNew = depth === 0 && lastSignificantChar !== '.' && wordKind !== null && (
        start < 0 || kind === 'shell' || (kind === 'sql' && !lineHasToken)
      );
      if (startsNew) {
        finish(lastSignificantEnd);
        start = i;
        kind = wordKind ?? 'shell';
      } else if (start < 0) {
        start = i;
        kind = 'shell';
      }
      i = j;
      lastSignificantEnd = j;
      lastSignificantChar = word[word.length - 1];
      lineHasToken = true;
      continue;
    }

    if (start < 0) {
      start = i;
      kind = 'shell';
    }
    lineHasToken = true;

    // Strings ('' escapes in SQL read as two adjacent strings, which is fine here)
    if (c === '"' || c === "'" || c === '`') {
      let j = i + 1;
      while (j < text.length && text[j] !== c) {
        if (text[j] === '\\' && kind !== 'sql') j++;
        else if (text[j] === '\n' && c !== '`') break;
        j++;
      }
      i = Math.min(j + 1, text.length);
      lastSignificantEnd = i;
      lastSignificantChar = c;
      continue;
    }

    // Regex literal (shell only, where a value can start)
    if (c === '/' && kind !== 'sql' && /^[(,:[{;]?$/.test(lastSignificantChar)) {
      let j = i + 1;
      let inClass = false;
      while (j < text.length && text[j] !== '\n') {
        if (text[j] === '\\') j++;
        else if (text[j] === '[') inClass = true;
        else if (text[j] === ']') inClass = false;
        else if (text[j] === '/' && !inClass) break;
        j++;
      }
      j++;
      while (j < text.length && /[a-z]/i.test(text[j])) j++;
      i = Math.min(j, text.length);
      lastSignificantEnd = i;
      lastSignificantChar = '/';
      continue;
    }

    if (c === '(' || c === '{' || c === '[') depth++;
    if (c === ')' || c === '}' || c === ']') depth = Math.max(0, depth - 1);

    i++;
    lastSignificantEnd = i;
    lastSignificantChar = c;
  }
  finish(lastSignificantEnd);

  return ranges.map(({ start: s, end: e }) => {
    const from = positionAt(text, s);
    const to = positionAt(text, e);
    return {
      statement: text.slice(s, e),
      start: s,
      end: e,
      startLine: from.line,
      startColumn: from.column,
      endLine: to.line,
      endColumn: to.column,
    };
  });
}

/** Offset of a 1-based Monaco position */
export function offsetAt(text: string, line: number, column: number): number {
  let offset = 0;
  for (let l = 1; l < line; l++) {
    const nl = text.indexOf('\n', offset);
    if (nl === -1) return text.length;
    offset = nl + 1;
  }
  return Math.min(offset + column - 1, text.length);
}

/**
 * The statement under the cursor. When the cursor sits between statements,
 * the closest preceding statement wins (matching "run what I just typed").
 */
export function findMongoStatementAtCursor(
  text: string,
  line: number,
  column: number
): (MongoStatement & { statementIndex: number }) | null {
  const statements = splitMongoStatements(text);
  if (statements.length === 0) return null;
  const cursor = offsetAt(text, line, column);

  let chosen = -1;
  for (let idx = 0; idx < statements.length; idx++) {
    const s = statements[idx];
    if (cursor >= s.start && cursor <= s.end) {
      chosen = idx;
      break;
    }
    if (s.start <= cursor) chosen = idx;
  }
  if (chosen === -1) chosen = 0;
  return { ...statements[chosen], statementIndex: chosen };
}

/** Short result-tab label, e.g. `users.find`, `show dbs`, `runCommand` */
export function getMongoStatementLabel(statement: string | null | undefined): string {
  if (!statement) return 'Result';
  const text = statement.trim();
  if (isSqlStatement(text)) return getResultStatementLabel(text) ?? 'Result';
  const show = text.match(/^show\s+(\w+)/);
  if (show) return `show ${show[1]}`;
  const use = text.match(/^use\s+([^\s;]+)/);
  if (use) return `use ${use[1]}`;

  const coll =
    text.match(/^db\.getCollection\(\s*["'`]([^"'`]+)["'`]\s*\)\s*\.\s*(\w+)/) ||
    text.match(/^db\[\s*["'`]([^"'`]+)["'`]\s*\]\s*\.\s*(\w+)/);
  if (coll) return `${coll[1]}.${coll[2]}`;

  const dotted = text.match(/^db((?:\s*\.\s*[\w$]+)+)\s*\(/);
  if (dotted) {
    const parts = dotted[1].split('.').map(p => p.trim()).filter(Boolean);
    return parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join('.')}.${parts[parts.length - 1]}`;
  }
  return text.split(/\s+/)[0] || 'Result';
}

/** `db.name`, or `db.getCollection("name")` when the name is not an identifier */
export function mongoCollectionExpression(collection: string): string {
  return /^[A-Za-z_$][\w$]*$/.test(collection)
    ? `db.${collection}`
    : `db.getCollection(${JSON.stringify(collection)})`;
}

/** Query that previews the first `limit` documents of a collection */
export function mongoFindQuery(collection: string, limit: number): string {
  return `${mongoCollectionExpression(collection)}.find({}).limit(${limit})`;
}

/** Whether a host field holds a MongoDB connection string */
export function isMongoConnectionString(host: string): boolean {
  return /^\s*mongodb(\+srv)?:\/\//i.test(host);
}

/** Database named in a connection string path (`mongodb://h:27017/shop?x=y` → `shop`), if any */
export function databaseFromConnectionString(uri: string): string | null {
  const match = uri.trim().match(/^mongodb(?:\+srv)?:\/\/([^/?#]*)\/([^?#]*)/i);
  if (!match || !match[2]) return null;
  try {
    return decodeURIComponent(match[2]) || null;
  } catch {
    return match[2];
  }
}
