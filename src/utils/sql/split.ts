/**
 * T-SQL statement splitter.
 * Handles: ; outside strings/brackets, '' doubling, [bracket;], -- and block comments, GO with optional count.
 */
export function splitSqlStatements(sql: string): string[] {
  const statements: string[] = [];
  let current = '';
  let i = 0;
  const len = sql.length;

  // State machine
  type State = 'NORMAL' | 'STRING' | 'BRACKET' | 'LINE_COMMENT' | 'BLOCK_COMMENT';
  let state: State = 'NORMAL';

  // For GO detection: track if we are at start of line (after \n or at 0) and in NORMAL
  let atLineStart = true;
  let lineBuffer = ''; // buffer for current line content to detect GO

  const flush = () => {
    const trimmed = current.trim();
    if (trimmed) statements.push(trimmed);
    current = '';
    lineBuffer = '';
  };

  while (i < len) {
    const ch = sql[i];
    const next = i + 1 < len ? sql[i + 1] : '';

    if (state === 'STRING') {
      current += ch;
      lineBuffer += ch;
      if (ch === "'") {
        if (next === "'") {
          // Escaped '' inside string
          current += next;
          lineBuffer += next;
          i += 2;
          continue;
        } else {
          state = 'NORMAL';
        }
      }
      i++;
      if (ch === '\n') {
        atLineStart = true;
        lineBuffer = '';
      } else {
        atLineStart = false;
      }
      continue;
    }

    if (state === 'BRACKET') {
      current += ch;
      lineBuffer += ch;
      if (ch === ']') {
        if (next === ']') {
          current += next;
          lineBuffer += next;
          i += 2;
          continue;
        } else {
          state = 'NORMAL';
        }
      }
      i++;
      continue;
    }

    if (state === 'LINE_COMMENT') {
      current += ch;
      lineBuffer += ch;
      if (ch === '\n') {
        state = 'NORMAL';
        atLineStart = true;
        lineBuffer = '';
      }
      i++;
      continue;
    }

    if (state === 'BLOCK_COMMENT') {
      current += ch;
      lineBuffer += ch;
      if (ch === '*' && next === '/') {
        current += next;
        lineBuffer += next;
        i += 2;
        state = 'NORMAL';
        continue;
      }
      if (ch === '\n') {
        atLineStart = true;
        lineBuffer = '';
      }
      i++;
      continue;
    }

    // NORMAL state
    if (ch === "'") {
      state = 'STRING';
      current += ch;
      lineBuffer += ch;
      atLineStart = false;
      i++;
      continue;
    }
    if (ch === '[') {
      state = 'BRACKET';
      current += ch;
      lineBuffer += ch;
      atLineStart = false;
      i++;
      continue;
    }
    if (ch === '-' && next === '-') {
      state = 'LINE_COMMENT';
      current += ch + next;
      lineBuffer += ch + next;
      i += 2;
      atLineStart = false;
      continue;
    }
    if (ch === '/' && next === '*') {
      state = 'BLOCK_COMMENT';
      current += ch + next;
      lineBuffer += ch + next;
      i += 2;
      atLineStart = false;
      continue;
    }
    if (ch === ';') {
      flush();
      i++;
      atLineStart = false;
      lineBuffer = '';
      continue;
    }

    // Check for GO at line start (case-insensitive, with optional count and comments)
    if (atLineStart && (ch === 'G' || ch === 'g')) {
      // Peek the rest of the line
      let j = i;
      let lineEnd = sql.indexOf('\n', j);
      if (lineEnd === -1) lineEnd = len;
      const line = sql.slice(j, lineEnd);
      // GO pattern: GO [0-9]* (\s*(--.*|/\*.*\*/))? \s* $
      const goMatch = line.match(/^GO(\s+\d+)?(\s+(--.*|\/\*.*\*\/))?\s*$/i);
      if (goMatch) {
        // Flush current before GO
        if (current.trim()) flush();
        // GO is a batch separator, not part of any statement
        i = lineEnd + 1; // move past newline (or len)
        current = '';
        lineBuffer = '';
        atLineStart = true;
        continue;
      }
    }

    current += ch;
    lineBuffer += ch;
    if (ch === '\n') {
      atLineStart = true;
      lineBuffer = '';
    } else if (ch.trim() !== '') {
      atLineStart = false;
    }
    i++;
  }

  if (current.trim()) flush();

  return statements;
}
