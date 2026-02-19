/**
 * Smart SQL query extraction using node-sql-parser
 * Detects which statement to execute based on cursor position
 */

import NodeSqlParser from 'node-sql-parser';

const { Parser } = NodeSqlParser;

export interface StatementLocation {
  /** The SQL statement text to execute */
  statement: string;
  /** 1-based line number where statement starts */
  startLine: number;
  /** 1-based column number where statement starts */
  startColumn: number;
  /** 1-based line number where statement ends */
  endLine: number;
  /** 1-based column number where statement ends */
  endColumn: number;
  /** Index of this statement in the batch (0-based) */
  statementIndex: number;
}

/**
 * Extract the SQL statement at the cursor position
 * @param sql - Full SQL text from the editor
 * @param cursorLine - 1-based line number of cursor
 * @param cursorColumn - 1-based column number of cursor
 * @returns Statement location or null if detection failed
 */
export function extractStatementAtCursor(
  sql: string,
  cursorLine: number,
  _cursorColumn: number
): StatementLocation | null {
  if (!sql || sql.trim().length === 0) {
    return null;
  }

  const batches = splitByGoBatches(sql);

  // Find which batch contains the cursor
  let currentLine = 1;
  let batchWithCursor: { batch: string; startLine: number } | null = null;

  for (const batch of batches) {
    const batchLines = batch.split('\n').length;
    const batchEndLine = currentLine + batchLines - 1;

    if (cursorLine >= currentLine && cursorLine <= batchEndLine) {
      batchWithCursor = { batch, startLine: currentLine };
      break;
    }

    currentLine = batchEndLine + 1;
  }

  if (!batchWithCursor) {
    return null;
  }

  const parser = new Parser();
  let statements: any[];

  try {
    // Try TransactSQL first
    try {
      const ast = parser.astify(batchWithCursor.batch, { database: 'TransactSQL' });
      statements = Array.isArray(ast) ? ast : [ast];
    } catch (tsqlErr) {
      // Fallback to MySQL
      const ast = parser.astify(batchWithCursor.batch, { database: 'MySQL' });
      statements = Array.isArray(ast) ? ast : [ast];
    }
  } catch (err) {
    // Parsing failed - return null to trigger fallback
    console.warn('Query parsing failed, will use fallback detection:', err);
    return null;
  }

  if (!statements || statements.length === 0) {
    return null;
  }

  const statementTexts = splitBySemicolon(batchWithCursor.batch);

  // If we have more statements in AST than text splits, something's wrong
  // Just use the whole batch
  if (statements.length !== statementTexts.length) {
    // Use the entire batch as one statement
    return {
      statement: batchWithCursor.batch,
      startLine: batchWithCursor.startLine,
      startColumn: 1,
      endLine: batchWithCursor.startLine + batchWithCursor.batch.split('\n').length - 1,
      endColumn: batchWithCursor.batch.split('\n').pop()!.length + 1,
      statementIndex: 0,
    };
  }

  const batchLines = batchWithCursor.batch.split('\n');
  let searchStartLine = 0; // 0-based index within batch

  for (let i = 0; i < statementTexts.length; i++) {
    const stmtText = statementTexts[i].trim();

    // Find where this statement starts in the batch (from searchStartLine onwards)
    let stmtStartLine = -1;
    let stmtEndLine = -1;

    // Search for the first non-empty line of the statement
    const stmtFirstLine = stmtText.split('\n')[0].trim();

    for (let lineIdx = searchStartLine; lineIdx < batchLines.length; lineIdx++) {
      if (batchLines[lineIdx].trim().indexOf(stmtFirstLine) !== -1) {
        stmtStartLine = lineIdx;
        break;
      }
    }

    if (stmtStartLine === -1) {
      // Couldn't find statement, skip
      continue;
    }

    // Find the end line by counting lines in the statement
    const stmtLineCount = stmtText.split('\n').length;
    stmtEndLine = stmtStartLine;

    let linesFound = 0;
    for (let lineIdx = stmtStartLine; lineIdx < batchLines.length && linesFound < stmtLineCount; lineIdx++) {
      if (batchLines[lineIdx].trim().length > 0 || linesFound > 0) {
        linesFound++;
        stmtEndLine = lineIdx;
      }
    }

    // Convert to absolute line numbers (1-based)
    const absoluteStartLine = batchWithCursor.startLine + stmtStartLine;
    const absoluteEndLine = batchWithCursor.startLine + stmtEndLine;

    // Check if cursor is within this statement
    if (cursorLine >= absoluteStartLine && cursorLine <= absoluteEndLine) {
      const firstLine = batchLines[stmtStartLine];
      const startColumn = firstLine.indexOf(stmtFirstLine) + 1;

      const stmtLastLineText = stmtText.split('\n').pop()!.trim();
      const lastLine = batchLines[stmtEndLine];
      const searchFrom = (stmtStartLine === stmtEndLine) ? (startColumn - 1) : 0;
      const endColumn = lastLine.indexOf(stmtLastLineText, searchFrom) + stmtLastLineText.length + 1;

      // Check if cursor is strictly within the columns for the start/end lines
      const isAfterStart = cursorLine > absoluteStartLine || (cursorLine === absoluteStartLine && _cursorColumn >= startColumn);
      const isBeforeEnd = cursorLine < absoluteEndLine || (cursorLine === absoluteEndLine && _cursorColumn <= endColumn);

      if (isAfterStart && isBeforeEnd) {
        return {
          statement: stmtText,
          startLine: absoluteStartLine,
          startColumn,
          endLine: absoluteEndLine,
          endColumn,
          statementIndex: i,
        };
      }
    }

    // Move search position past this statement
    searchStartLine = stmtEndLine + 1;
  }

  // Cursor is after all statements - return the last statement
  if (statementTexts.length > 0) {
    const lastStmt = statementTexts[statementTexts.length - 1].trim();

    // Find the last statement in the batch
    const lastStmtFirstLine = lastStmt.split('\n')[0].trim();
    let lastStmtStartLine = -1;

    for (let lineIdx = batchLines.length - 1; lineIdx >= 0; lineIdx--) {
      if (batchLines[lineIdx].trim().indexOf(lastStmtFirstLine) !== -1) {
        lastStmtStartLine = lineIdx;
        break;
      }
    }

    if (lastStmtStartLine !== -1) {
      const stmtLineCount = lastStmt.split('\n').length;
      const lastStmtEndLine = Math.min(lastStmtStartLine + stmtLineCount - 1, batchLines.length - 1);

      const absoluteStartLine = batchWithCursor.startLine + lastStmtStartLine;
      const absoluteEndLine = batchWithCursor.startLine + lastStmtEndLine;

      const firstLine = batchLines[lastStmtStartLine];
      const startColumn = firstLine.indexOf(lastStmtFirstLine) + 1;
      const lastLine = batchLines[lastStmtEndLine];
      const endColumn = lastLine.length + 1;

      return {
        statement: lastStmt,
        startLine: absoluteStartLine,
        startColumn,
        endLine: absoluteEndLine,
        endColumn,
        statementIndex: statementTexts.length - 1,
      };
    }
  }

  return null;
}

/**
 * Split SQL by GO batch separator (case-insensitive, must be on its own line)
 * GO is SQL Server specific and not valid SQL syntax
 */
function splitByGoBatches(sql: string): string[] {
  const lines = sql.split('\n');
  const batches: string[] = [];
  let currentBatch: string[] = [];

  for (const line of lines) {
    const trimmed = line.trim();

    if (/^GO$/i.test(trimmed)) {
      if (currentBatch.length > 0) {
        batches.push(currentBatch.join('\n'));
        currentBatch = [];
      }
      // Don't include the GO line itself
    } else {
      currentBatch.push(line);
    }
  }

  // Add final batch
  if (currentBatch.length > 0) {
    batches.push(currentBatch.join('\n'));
  }

  return batches.length > 0 ? batches : [sql];
}

function splitBySemicolon(sql: string): string[] {
  const statements: string[] = [];
  let currentStatement = '';
  let inString: string | null = null;
  let inBracket = false;
  let inLineComment = false;
  let inBlockComment = false;

  for (let i = 0; i < sql.length; i++) {
    const char = sql[i];
    const nextChar = i < sql.length - 1 ? sql[i + 1] : '';

    if (!inString && !inBlockComment && char === '-' && nextChar === '-') {
      inLineComment = true;
      currentStatement += char;
      continue;
    }

    if (inLineComment && char === '\n') {
      inLineComment = false;
      currentStatement += char;
      continue;
    }

    if (!inString && !inLineComment && char === '/' && nextChar === '*') {
      inBlockComment = true;
      currentStatement += char;
      continue;
    }

    if (inBlockComment && char === '*' && nextChar === '/') {
      inBlockComment = false;
      currentStatement += char + nextChar;
      i++;
      continue;
    }

    if (inLineComment || inBlockComment) {
      currentStatement += char;
      continue;
    }

    // Handle square brackets
    if (!inString && char === '[') {
      inBracket = true;
      currentStatement += char;
      continue;
    }

    if (inBracket && char === ']') {
      inBracket = false;
      currentStatement += char;
      continue;
    }

    if (!inBracket && (char === "'" || char === '"')) {
      if (!inString) {
        inString = char;
      } else if (inString === char) {
        if (nextChar === char) {
          currentStatement += char + nextChar;
          i++;
          continue;
        } else {
          inString = null;
        }
      }
      currentStatement += char;
      continue;
    }

    if (!inString && !inBracket && char === ';') {
      const stmt = currentStatement.trim();
      if (stmt.length > 0) {
        statements.push(stmt);
      }
      currentStatement = '';
      continue;
    }

    // Regular character
    currentStatement += char;
  }

  // Add final statement if any
  const finalStmt = currentStatement.trim();
  if (finalStmt.length > 0) {
    statements.push(finalStmt);
  }

  return statements.length > 0 ? statements : [sql];
}

/**
 * Fallback function for when parsing fails
 * Uses simple regex-based detection (existing logic from QueryEditor)
 */
/**
 * Extract all SQL statements from the text
 * @param sql - Full SQL text
 * @returns Array of statement locations
 */
export function extractAllStatements(sql: string): StatementLocation[] {
  if (!sql || sql.trim().length === 0) {
    return [];
  }

  const result: StatementLocation[] = [];

  // Step 1: Handle GO batch separator
  const batches = splitByGoBatches(sql);
  let currentLine = 1;

  for (const batch of batches) {
    const batchLines = batch.split('\n');
    const batchLineCount = batchLines.length;

    if (batch.trim().length === 0) {
      currentLine += batchLineCount + (batches.length > 1 ? 1 : 0);
      continue;
    }

    const statementTexts = splitBySemicolon(batch);

    let searchStartLine = 0; // 0-based index within batch

    for (let i = 0; i < statementTexts.length; i++) {
      const stmtText = statementTexts[i].trim();
      if (stmtText.length === 0) continue;

      let stmtStartLine = -1;
      let stmtEndLine = -1;
      const stmtFirstLine = stmtText.split('\n')[0].trim();

      for (let lineIdx = searchStartLine; lineIdx < batchLines.length; lineIdx++) {
        if (batchLines[lineIdx].trim().indexOf(stmtFirstLine) !== -1) {
          stmtStartLine = lineIdx;
          break;
        }
      }

      if (stmtStartLine === -1) continue;

      const stmtLineCount = stmtText.split('\n').length;
      stmtEndLine = stmtStartLine;

      let linesFound = 0;
      for (let lineIdx = stmtStartLine; lineIdx < batchLines.length && linesFound < stmtLineCount; lineIdx++) {
        stmtEndLine = lineIdx;
        linesFound++;
      }

      const absoluteStartLine = currentLine + stmtStartLine;
      const absoluteEndLine = currentLine + stmtEndLine;

      const firstLine = batchLines[stmtStartLine];
      const startColumn = firstLine.indexOf(stmtFirstLine) + 1;
      const lastLine = batchLines[stmtEndLine];
      const endColumn = lastLine.length + 1;

      result.push({
        statement: stmtText,
        startLine: absoluteStartLine,
        startColumn: startColumn > 0 ? startColumn : 1,
        endLine: absoluteEndLine,
        endColumn,
        statementIndex: result.length
      });

      searchStartLine = stmtEndLine + 1;
    }

    currentLine += batchLines.length;
    if (batches.length > 1) {
      currentLine += 1;
    }
  }

  return result;
}

/**
 * Fallback function for when parsing fails
 * Uses simple regex-based detection (existing logic from QueryEditor)
 */
export function findCurrentSqlBlockFallback(
  sql: string,
  cursorLine: number,
  _cursorColumn: number
): StatementLocation | null {
  const lines = sql.split('\n');

  let startLine = cursorLine;
  for (let i = cursorLine - 2; i >= 0; i--) {
    const line = lines[i].trim();

    if (cursorLine - i > 100) break;

    if (line.length === 0 || /^GO$/i.test(line) || line.endsWith(';')) {
      startLine = i + 2;
      break;
    }

    startLine = i + 1;
  }



  let endLine = cursorLine;
  for (let i = cursorLine - 1; i < lines.length; i++) {
    const line = lines[i].trim();

    if (i - cursorLine > 100) break;

    if (/^GO$/i.test(line) || line.endsWith(';')) {
      endLine = i + 1;
      break;
    }

    // Boundary detection based on new statement keywords or empty lines
    const isNewStatement = i > cursorLine - 1 && /^(SELECT|INSERT|UPDATE|DELETE|CREATE|ALTER|DROP|WITH|DECLARE|EXEC)\b/i.test(line);
    if (isNewStatement || (line.length === 0 && i >= cursorLine)) {
      endLine = i;
      break;
    }

    endLine = i + 1;
  }

  const statementLines = lines.slice(startLine - 1, endLine);
  const statement = statementLines.join('\n').trim();

  if (statement.length === 0) return null;

  const firstLine = lines[startLine - 1] || '';
  const firstLineTrimmed = firstLine.trimStart();
  const startColumn = (firstLine.length - firstLineTrimmed.length) + 1;

  const lastLine = lines[endLine - 1] || '';
  const endColumn = lastLine.length + 1;

  return {
    statement,
    startLine,
    startColumn,
    endLine,
    endColumn,
    statementIndex: 0,
  };
}
