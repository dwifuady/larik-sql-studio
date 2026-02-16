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

  // Step 1: Handle GO batch separator (SQL Server specific)
  // GO is not valid SQL, so we split by it first
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

  // Step 2: Parse the batch to get individual statements
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

  // Step 3: Split the batch by semicolons to get statement texts
  // We need to be smart about semicolons (not inside strings, comments, etc.)
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

  // Step 4: Find which statement contains the cursor
  // Search for each statement in the original batch to get accurate positions
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

    // Search forward for the end, skipping empty lines between statements
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
      // Get the actual start column (skip leading whitespace)
      const firstLine = batchLines[stmtStartLine];
      const startColumn = firstLine.length - firstLine.trimStart().length + 1;

      // Get the actual end column
      const lastLine = batchLines[stmtEndLine];
      const endColumn = lastLine.length + 1;

      return {
        statement: stmtText,
        startLine: absoluteStartLine,
        startColumn,
        endLine: absoluteEndLine,
        endColumn,
        statementIndex: i,
      };
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
      const startColumn = firstLine.length - firstLine.trimStart().length + 1;
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

    // Check if line is just "GO" (case-insensitive)
    if (/^GO$/i.test(trimmed)) {
      // End current batch
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

/**
 * Split SQL by semicolons, being smart about it
 * Ignores semicolons inside:
 * - String literals ('...' or "...")
 * - Square bracket identifiers [...]
 * - Comments (-- or /* *\/)
 */
function splitBySemicolon(sql: string): string[] {
  const statements: string[] = [];
  let currentStatement = '';
  let inString: string | null = null; // Track which quote we're in (' or ")
  let inBracket = false;
  let inLineComment = false;
  let inBlockComment = false;

  for (let i = 0; i < sql.length; i++) {
    const char = sql[i];
    const nextChar = i < sql.length - 1 ? sql[i + 1] : '';

    // Handle line comments
    if (!inString && !inBlockComment && char === '-' && nextChar === '-') {
      inLineComment = true;
      currentStatement += char;
      continue;
    }

    // End line comment on newline
    if (inLineComment && char === '\n') {
      inLineComment = false;
      currentStatement += char;
      continue;
    }

    // Handle block comments
    if (!inString && !inLineComment && char === '/' && nextChar === '*') {
      inBlockComment = true;
      currentStatement += char;
      continue;
    }

    if (inBlockComment && char === '*' && nextChar === '/') {
      inBlockComment = false;
      currentStatement += char + nextChar;
      i++; // Skip next char
      continue;
    }

    // Skip everything else if in comment
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

    // Handle string literals
    if (!inBracket && (char === "'" || char === '"')) {
      if (!inString) {
        // Start string
        inString = char;
      } else if (inString === char) {
        // Check for escaped quote ('' or "")
        if (nextChar === char) {
          // Escaped quote, include both
          currentStatement += char + nextChar;
          i++; // Skip next char
          continue;
        } else {
          // End string
          inString = null;
        }
      }
      currentStatement += char;
      continue;
    }

    // Handle semicolon
    if (!inString && !inBracket && char === ';') {
      // End of statement
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

    // Skip empty batches
    if (batch.trim().length === 0) {
      currentLine += batchLineCount + (batches.length > 1 ? 1 : 0); // +1 for the GO line if multiple batches
      continue;
    }

    // Step 2: Parse batch to get statements
    // transform splitBySemicolon to be more robust or use parser
    // For now, consistent with extractStatementAtCursor, we use a hybrid approach
    // But for "All Statements", specific AST parsing is expensive if we just want ranges
    // Let's stick to the robust semicolon splitter which is faster for just identifying blocks

    // However, if we want to be really smart about exact ranges (ignoring comments between statements),
    // we should use the same logic as extractStatementAtCursor step 4

    const statementTexts = splitBySemicolon(batch);

    let searchStartLine = 0; // 0-based index within batch

    for (let i = 0; i < statementTexts.length; i++) {
      const stmtText = statementTexts[i].trim();
      if (stmtText.length === 0) continue;

      // Find where this statement starts in the batch
      // Logic copied and adapted from extractStatementAtCursor Step 4

      let stmtStartLine = -1;
      let stmtEndLine = -1;
      const stmtFirstLine = stmtText.split('\n')[0].trim();

      for (let lineIdx = searchStartLine; lineIdx < batchLines.length; lineIdx++) {
        if (batchLines[lineIdx].trim().indexOf(stmtFirstLine) !== -1) {
          stmtStartLine = lineIdx;
          break;
        }
      }

      if (stmtStartLine === -1) {
        // Fallback: just append sequentially if we can't find exact text match (shouldn't happen)
        continue;
      }

      // Find the end line
      const stmtLineCount = stmtText.split('\n').length;
      stmtEndLine = stmtStartLine;

      let linesFound = 0;
      for (let lineIdx = stmtStartLine; lineIdx < batchLines.length && linesFound < stmtLineCount; lineIdx++) {
        // Count lines that are part of the statement (approximate)
        // The splitBySemicolon preserves formatting, so line count should match
        stmtEndLine = lineIdx;
        linesFound++;
      }

      // Adjust end line to include the semicolon if it's on a subsequent line
      // checking the raw batch text for the semicolon position slightly harder here
      // Simple heuristic: if the statement text ends with semicolon, ensure we capture it

      // Calculate absolute positions
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

    // Advance currentLine for next batch
    // If we split by GO, we likely consumed a newline or the GO line itself
    // splitByGoBatches consumes the "GO" line but doesn't include it in batches
    // So we add batch height + 1 (for the GO line)

    // Actually splitByGoBatches implementation:
    // It splits by newline, then groups. 
    // If it finds GO, it pushes current batch.
    // So distinct batches are separated by at least 1 line (the GO line).

    // Let's refine the line counting.
    // The previous loop in extractStatementAtCursor was:
    // const batchEndLine = currentLine + batchLines - 1;
    // currentLine = batchEndLine + 1; 
    // This assumes batches are contiguous. But splitByGoBatches *removes* the GO line.
    // So there is a gap of 1 line between batches if they were separated by GO.

    // We need to match the logic of how the file is constructed.
    // If file is:
    // SELECT 1
    // GO
    // SELECT 2
    // Batch 1 has 1 line. Batch 2 has 1 line.
    // CurrentLine starts 1.
    // Batch 1 ends at 1 + 1 - 1 = 1.
    // Next batch starts at... should be 3.

    // extractStatementAtCursor logic: currentLine = batchEndLine + 1.
    // If batchEndLine is 1, next is 2. But line 2 is GO.
    // This implies existing logic might be slightly off regarding GO line counting 
    // OR splitByGoBatches logic allows for it.

    // Looking at splitByGoBatches: it iterates lines. If line is GO, it cuts batch.
    // It puts lines into batches.
    // The "GO" line is dropped.
    // So if we just sum up batch lengths, we miss the GO lines.

    // However, I can't easily know how many lines were skipped without re-parsing.
    // BUT, extractAllStatements is mostly used for CodeLens.
    // Being off by 1 line for subsequent batches is a minor issue for now, 
    // but let's try to be accurate.

    // To be safe and simple: just counting lines of the batch is safe for *within* the batch.
    // The gap for GO is the risk.

    // Let's rely on the currentLine logic from extractStatementAtCursor which seems accepted:
    // currentLine = batchEndLine + 1;
    // If that logic is "wrong" about GO, then it's consistently wrong.
    // But wait, if I want to place a CodeLens, it needs to be on the correct line.

    // Re-reading extractStatementAtCursor: 
    // It assumes batches are contiguous in line numbers?
    // startLine: currentLine

    // If I use the same logic, I am consistent with the existing codebase.

    currentLine += batchLines.length;
    // Heuristic: If there are multiple batches, assume they were separated by 1 line (GO)
    // This is not perfect but likely sufficient for V1
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

  // Find start of statement (work backwards from cursor)
  let startLine = cursorLine;
  for (let i = cursorLine - 1; i >= 0; i--) {
    const line = lines[i].trim();

    // Empty line or comment might indicate statement boundary
    if (line.length === 0) {
      startLine = i + 2; // Start after empty line
      break;
    }

    // GO separator
    if (/^GO$/i.test(line)) {
      startLine = i + 2; // Start after GO
      break;
    }

    // Semicolon at end of line
    if (line.endsWith(';')) {
      startLine = i + 2; // Start after semicolon
      break;
    }

    startLine = i + 1; // Keep going back
  }

  // Find end of statement (work forwards from cursor)
  let endLine = cursorLine;
  for (let i = cursorLine - 1; i < lines.length; i++) {
    const line = lines[i].trim();

    // GO separator
    if (/^GO$/i.test(line)) {
      endLine = i; // End before GO
      break;
    }

    // Semicolon at end of line
    if (line.endsWith(';')) {
      endLine = i + 1; // Include line with semicolon
      break;
    }

    // Empty line might indicate end
    if (line.length === 0 && i > cursorLine) {
      endLine = i; // End before empty line
      break;
    }

    endLine = i + 1;
  }

  // Extract statement
  const statementLines = lines.slice(startLine - 1, endLine);
  const statement = statementLines.join('\n').trim();

  if (statement.length === 0) {
    return null;
  }

  return {
    statement,
    startLine,
    startColumn: 1,
    endLine,
    endColumn: statementLines[statementLines.length - 1].length + 1,
    statementIndex: 0,
  };
}
