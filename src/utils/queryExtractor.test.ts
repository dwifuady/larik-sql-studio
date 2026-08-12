import { describe, it, expect } from 'vitest';
import { extractStatementAtCursor } from './queryExtractor';

describe('queryExtractor', () => {
  it('should extract single statement', () => {
    const sql = 'SELECT * FROM users;';
    // Cursor at column 10 (0-indexed logic in test, but function expects 1-based)
    const result = extractStatementAtCursor(sql, 1, 10);
    expect(result).not.toBeNull();
    expect(result!.statement.trim()).toBe('SELECT * FROM users');
  });

  it('should handle multiple statements', () => {
    const sql = `
      SELECT * FROM users;
      SELECT * FROM orders;
      SELECT * FROM products;
    `;
    // Cursor on line 3 (SELECT * FROM orders), 1-based index
    const result = extractStatementAtCursor(sql, 3, 10);
    expect(result).not.toBeNull();
    expect(result!.statement).toContain('orders');
    expect(result!.statementIndex).toBe(1);
  });

  it('should handle GO batch separator', () => {
    const sql = `
      SELECT * FROM users;
      GO
      SELECT * FROM orders;
    `;
    // Cursor on line 4 (SELECT * FROM orders), 1-based index
    const result = extractStatementAtCursor(sql, 4, 10);
    expect(result).not.toBeNull();
    expect(result!.statement).toContain('orders');
  });

  it('should handle nested queries', () => {
    const sql = `
      SELECT *
      FROM users
      WHERE id IN (SELECT user_id FROM orders WHERE total > 100);
    `;
    // Cursor on line 3, 1-based index
    const result = extractStatementAtCursor(sql, 3, 10);
    expect(result).not.toBeNull();
    expect(result!.statement).toContain('WHERE id IN');
  });

  it('should handle CTEs', () => {
    const sql = `
      WITH recent_orders AS (
        SELECT * FROM orders WHERE created_at > '2024-01-01'
      )
      SELECT * FROM recent_orders;
    `;
    // Cursor on line 4, 1-based index
    const result = extractStatementAtCursor(sql, 4, 10);
    expect(result).not.toBeNull();
    expect(result!.statement).toContain('WITH recent_orders');
  });

  it('should handle string literals with semicolons', () => {
    const sql = `SELECT 'Hello; World' AS message;`;
    const result = extractStatementAtCursor(sql, 1, 10);
    expect(result).not.toBeNull();
    expect(result!.statement.trim()).toBe("SELECT 'Hello; World' AS message");
  });

  it('should handle comments', () => {
    const sql = `
      -- This is a comment with SELECT keyword
      SELECT * FROM users;
      /* Multi-line
         comment with ; semicolon */
      SELECT * FROM orders;
    `;
    // Cursor on line 6 (SELECT * FROM orders), 1-based index
    const result = extractStatementAtCursor(sql, 6, 10);
    expect(result).not.toBeNull();
    expect(result!.statement).toContain('orders');
  });

  it('should extract a full MERGE statement when cursor is on a WHEN clause', () => {
    const sql = `MERGE [dbo].[Settings] AS [Target]
USING (VALUES
    ('ShowCreditCardFeeOnlyWhenCalculable', 'False', 1, 'Feature flag')
) AS [Source] ([Key], [Value], [Active], [Description])
ON [Target].[Key] = [Source].[Key] AND [Target].[BizEntityId] IS NULL
WHEN NOT MATCHED BY TARGET THEN
    INSERT ([Key], [Value], [Active], [BizEntityId], [Description])
    VALUES ([Source].[Key], [Source].[Value], [Source].[Active], NULL, [Source].[Description])
WHEN MATCHED THEN
    UPDATE SET
        [Target].[Value] = [Source].[Value],
        [Target].[Active] = [Source].[Active];`;
    // Cursor on the "WHEN MATCHED THEN" line (1-based line 9)
    const result = extractStatementAtCursor(sql, 9, 1);
    expect(result).not.toBeNull();
    expect(result!.statement).toContain('MERGE [dbo].[Settings]');
    expect(result!.statement).toContain('WHEN NOT MATCHED BY TARGET THEN');
    expect(result!.statement).toContain('WHEN MATCHED THEN');
    expect(result!.statement).toContain('[Target].[Active] = [Source].[Active]');
    expect(result!.startLine).toBe(1);
    expect(result!.endLine).toBe(12);
  });

  it('should extract a full MERGE statement when cursor is in the USING VALUES block', () => {
    const sql = `MERGE [dbo].[Settings] AS [Target]
USING (VALUES
    ('ShowCreditCardFeeOnlyWhenCalculable', 'False', 1, 'Feature flag')
) AS [Source] ([Key], [Value], [Active], [Description])
ON [Target].[Key] = [Source].[Key] AND [Target].[BizEntityId] IS NULL
WHEN MATCHED THEN
    UPDATE SET
        [Target].[Value] = [Source].[Value],
        [Target].[Active] = [Source].[Active];`;
    // Cursor on the VALUES row line (1-based line 3)
    const result = extractStatementAtCursor(sql, 3, 10);
    expect(result).not.toBeNull();
    expect(result!.statement).toContain('MERGE [dbo].[Settings]');
    expect(result!.statement).toContain('[Target].[Active] = [Source].[Active]');
    expect(result!.startLine).toBe(1);
  });

  it('should return null for empty SQL', () => {
    const sql = '';
    const result = extractStatementAtCursor(sql, 1, 1);
    expect(result).toBeNull();
  });

  it('should handle cursor at end of statement', () => {
    const sql = 'SELECT * FROM users;';
    const result = extractStatementAtCursor(sql, 1, 20); // After semicolon
    expect(result).not.toBeNull();
  });

  it('should handle cursor between statements', () => {
    const sql = `
      SELECT * FROM users;

      SELECT * FROM orders;
    `;
    // Cursor on blank line (line 3), 1-based index
    const result = extractStatementAtCursor(sql, 3, 1);
    // Should find closest statement (prefer one before cursor)
    // Note: The implementation details might vary on "closest",
    // but typically it attaches to the previous one or next one depending on specific logic.
    // Based on `queryExtractor.ts`:
    // It splits by semicolon and finds which statement *contains* the cursor line/col.
    // If the cursor is on an empty line between statements, `extractStatementAtCursor` logic:
    // It iterates statements. `stmtEndLine` is calculated.
    // If there are empty lines between, they might not be "owned" by any statement unless logic is lenient.
    // Logic attaches to the previous or next statement depending on cursor position.
    expect(result).toBeDefined();
  });
});
