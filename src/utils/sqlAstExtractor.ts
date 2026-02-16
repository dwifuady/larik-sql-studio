/**
 * SQL AST-based extraction utilities using node-sql-parser
 * These functions replace regex-based implementations with robust AST parsing
 */

import NodeSqlParser from 'node-sql-parser';

const { Parser } = NodeSqlParser;

/**
 * Completion context types
 */
export type CompletionContextType =
  | 'keyword'
  | 'table'
  | 'column'
  | 'alias_column'
  | 'schema'
  | 'routine'
  | 'function'
  | 'database'
  | 'update_column'
  | 'insert_column';

/**
 * Completion context result
 */
export interface CompletionContext {
  type: CompletionContextType;
  alias?: string;
  lastKeyword?: string;
  partialWord?: string;
  targetTable?: { schema: string; table: string };
  isInSubquery?: boolean;
  isInCTE?: boolean;
  statementType?: 'select' | 'insert' | 'update' | 'delete' | 'create' | 'alter' | 'drop';
}

/**
 * Normalize SQL Server bracketed identifiers to standard identifiers
 * Converts [schema].[table] to schema.table or "schema"."table" for better parser compatibility
 */
function normalizeSqlServerIdentifiers(sql: string): string {
  // Replace [identifier] with identifier (unquoted)
  // This handles: [dbo].[Users], [Column Name], etc.
  // But preserves brackets inside strings

  let result = '';
  let inString = false;
  let stringChar = '';
  let i = 0;

  while (i < sql.length) {
    const char = sql[i];
    const nextChar = i < sql.length - 1 ? sql[i + 1] : '';

    // Track string literals
    if ((char === "'" || char === '"') && !inString) {
      inString = true;
      stringChar = char;
      result += char;
      i++;
      continue;
    }

    if (inString && char === stringChar) {
      // Check for escaped quote
      if (nextChar === stringChar) {
        result += char + nextChar;
        i += 2;
        continue;
      }
      inString = false;
      result += char;
      i++;
      continue;
    }

    // If we're in a string, just copy the character
    if (inString) {
      result += char;
      i++;
      continue;
    }

    // Handle bracketed identifiers outside of strings
    if (char === '[') {
      // Find the closing bracket
      let j = i + 1;
      let identifier = '';
      while (j < sql.length && sql[j] !== ']') {
        identifier += sql[j];
        j++;
      }

      if (j < sql.length && sql[j] === ']') {
        // Successfully found closing bracket
        // Check if identifier needs quoting (has spaces, special chars, or is a keyword)
        const needsQuoting = /[\s-]/.test(identifier) || isReservedKeyword(identifier);

        if (needsQuoting) {
          result += `"${identifier}"`;
        } else {
          result += identifier;
        }

        i = j + 1; // Skip past the closing bracket
        continue;
      }
    }

    result += char;
    i++;
  }

  return result;
}

/**
 * Check if a word is a SQL reserved keyword that needs quoting
 */
function isReservedKeyword(word: string): boolean {
  const keywords = new Set([
    'SELECT', 'FROM', 'WHERE', 'JOIN', 'LEFT', 'RIGHT', 'INNER', 'OUTER',
    'ON', 'AND', 'OR', 'NOT', 'IN', 'EXISTS', 'BETWEEN', 'LIKE',
    'ORDER', 'GROUP', 'BY', 'HAVING', 'LIMIT', 'OFFSET',
    'INSERT', 'UPDATE', 'DELETE', 'CREATE', 'DROP', 'ALTER',
    'TABLE', 'INDEX', 'VIEW', 'DATABASE', 'SCHEMA',
    'AS', 'WITH', 'CASE', 'WHEN', 'THEN', 'ELSE', 'END',
    'DISTINCT', 'ALL', 'ANY', 'SOME', 'TOP', 'UNIQUE',
    'PRIMARY', 'FOREIGN', 'KEY', 'REFERENCES', 'CHECK', 'DEFAULT',
    'NULL', 'VALUES', 'SET', 'INTO', 'UNION', 'EXCEPT', 'INTERSECT'
  ]);

  return keywords.has(word.toUpperCase());
}

/**
 * Parse SQL and return AST, with multiple fallback strategies
 * Tries different normalization approaches and parser dialects
 */
function parseSQL(sql: string): any {
  // Empty or whitespace-only queries can't be parsed
  if (!sql || sql.trim().length === 0) {
    return null;
  }

  // Check if SQL contains a WITH clause (CTE) - we should still try to parse these
  // even if incomplete, to extract CTE names for autocomplete
  const hasWith = /\bWITH\b/i.test(sql);

  // Don't try to parse obviously incomplete SQL (reduces console noise during typing)
  // UNLESS it has a WITH clause - we want to extract CTEs even from incomplete queries
  const endsWithIncompleteKeyword = /\b(SELECT|FROM|WHERE|JOIN|LEFT|RIGHT|INNER|OUTER|ON|AND|OR|ORDER|GROUP|BY|SET|UPDATE|INSERT|INTO|VALUES)\s*$/i.test(sql.trim());

  if (endsWithIncompleteKeyword && !hasWith) {
    // Incomplete SQL without CTEs, skip parsing (will use regex fallback)
    return null;
  }

  const parser = new Parser();

  // Strategy 1: Try TransactSQL with normalized identifiers (SQL Server syntax)
  const normalizedSql = normalizeSqlServerIdentifiers(sql);

  try {
    const ast = parser.astify(normalizedSql, { database: 'TransactSQL' });
    if (ast) {
      return ast;
    }
  } catch (tsqlErr: any) {
    // Silent - try next strategy
  }

  // Strategy 2: Try MySQL with normalized identifiers (more permissive)
  try {
    const ast = parser.astify(normalizedSql, { database: 'MySQL' });
    if (ast) {
      return ast;
    }
  } catch (mysqlErr: any) {
    // Silent - try next strategy
  }

  // Strategy 3: Try TransactSQL with less aggressive normalization
  // Remove only the most problematic brackets (schema.table format)
  const lightlyNormalizedSql = sql
    .replace(/\[(\w+)\]\.\[(\w+)\]/g, '$1.$2')  // [schema].[table] -> schema.table
    .replace(/\[(\w+)\]\.(\w+)/g, '$1.$2')       // [schema].table -> schema.table
    .replace(/(\w+)\.\[(\w+)\]/g, '$1.$2');       // schema.[table] -> schema.table

  try {
    const ast = parser.astify(lightlyNormalizedSql, { database: 'TransactSQL' });
    if (ast) {
      return ast;
    }
  } catch (tsqlErr2: any) {
    // Silent - try next strategy
  }

  // Strategy 4: Try MySQL with less aggressive normalization
  try {
    const ast = parser.astify(lightlyNormalizedSql, { database: 'MySQL' });
    if (ast) {
      return ast;
    }
  } catch (mysqlErr2: any) {
    // All strategies exhausted
  }

  // Strategy 5: For queries with CTEs that end incompletely, try completing them
  // e.g., "WITH cte AS (...) SELECT * FROM " -> "WITH cte AS (...) SELECT * FROM dummy"
  if (hasWith && endsWithIncompleteKeyword) {
    const completedSql = sql.trim() + ' __dummy_table__';
    const normalizedCompleted = normalizeSqlServerIdentifiers(completedSql);

    // Try multiple dialects
    try {
      const ast = parser.astify(normalizedCompleted, { database: 'MySQL' });
      if (ast) {
        return ast;
      }
    } catch (err: any) {
      // Silent - try next dialect
    }

    try {
      const ast = parser.astify(normalizedCompleted, { database: 'TransactSQL' });
      if (ast) {
        return ast;
      }
    } catch (err: any) {
      // Silent - all strategies exhausted
    }
  }

  // If all parsing attempts fail, return null to trigger fallback
  // Only log for complete-looking SQL (to avoid noise during typing)
  if (sql.length > 50 && !endsWithIncompleteKeyword && !hasWith) {
    console.log('[AST Parser] Parse failed, using regex fallback for:', sql.substring(0, 100));
  }

  return null;
}

/**
 * Get all aliases used in the SQL query (including CTEs, table aliases, subquery aliases)
 *
 * @param sql - The SQL query text
 * @returns Set of alias names (lowercase)
 *
 * @example
 * getUsedAliases('SELECT * FROM Users u, Orders o')
 * // Returns: Set(['u', 'o'])
 *
 * getUsedAliases('WITH cte AS (...) SELECT * FROM cte c')
 * // Returns: Set(['cte', 'c'])
 */
export function getUsedAliases(sql: string): Set<string> {
  const aliases = new Set<string>();

  if (!sql || !sql.trim()) {
    return aliases;
  }

  const ast = parseSQL(sql);

  if (!ast) {
    // Fallback: use regex to match only table aliases (after FROM/JOIN keywords)
    // This pattern looks for FROM/JOIN followed by table name and alias
    // Matches: FROM Users AS u, JOIN Orders o, FROM [dbo].[Users] u, etc.
    const tableAliasPattern = /(?:FROM|JOIN|,)\s+(?:\[?\w+\]?\.)?\[?\w+\]?\s+(?:AS\s+)?\[?(\w+)\]?(?=\s|$|,|\)|;|\r|\n|WHERE|ON|JOIN|LEFT|RIGHT|INNER|OUTER|CROSS|ORDER|GROUP)/gi;
    let match;
    while ((match = tableAliasPattern.exec(sql)) !== null) {
      if (match[1]) {
        const alias = match[1];
        // Skip SQL keywords that might be matched
        const skipWords = new Set(['WHERE', 'ON', 'JOIN', 'LEFT', 'RIGHT', 'INNER', 'OUTER', 'CROSS', 'ORDER', 'GROUP', 'HAVING', 'UNION', 'SET', 'AND', 'OR', 'AS', 'SELECT', 'FROM', 'INTO', 'VALUES', 'UPDATE', 'DELETE', 'INSERT']);
        if (!skipWords.has(alias.toUpperCase())) {
          aliases.add(alias.toLowerCase());
        }
      }
    }
    return aliases;
  }

  // Handle multiple statements
  const statements = Array.isArray(ast) ? ast : [ast];

  for (const stmt of statements) {
    extractAliasesFromStatement(stmt, aliases);
  }

  return aliases;
}

/**
 * Recursively extract aliases from an AST statement
 */
function extractAliasesFromStatement(stmt: any, aliases: Set<string>): void {
  if (!stmt || typeof stmt !== 'object') {
    return;
  }

  // Extract CTE aliases (WITH clause)
  if (stmt.with && Array.isArray(stmt.with)) {
    for (const cte of stmt.with) {
      if (cte.name) {
        // CTE name might be a string or an object
        const cteName = typeof cte.name === 'string' ? cte.name : (cte.name.value || cte.name);
        if (typeof cteName === 'string') {
          aliases.add(cteName.toLowerCase());
        }
      }
    }
  }

  // Extract table aliases from FROM clause
  if (stmt.from && Array.isArray(stmt.from)) {
    for (const tableRef of stmt.from) {
      extractAliasesFromTableRef(tableRef, aliases);
    }
  }

  // Extract table aliases from JOIN clauses (already in from array)
  // Extract aliases from UPDATE/DELETE table references
  if (stmt.table && Array.isArray(stmt.table)) {
    for (const tableRef of stmt.table) {
      extractAliasesFromTableRef(tableRef, aliases);
    }
  }

  // Extract aliases from subqueries in SELECT columns
  if (stmt.columns && Array.isArray(stmt.columns)) {
    for (const col of stmt.columns) {
      if (col.expr && col.expr.type === 'select') {
        extractAliasesFromStatement(col.expr, aliases);
      }
      // NOTE: We don't extract column aliases (col.as) here because column aliases
      // don't conflict with table aliases in SQL. We only track table/CTE/subquery aliases.
    }
  }

  // Extract aliases from WHERE clause subqueries
  if (stmt.where) {
    extractAliasesFromExpression(stmt.where, aliases);
  }
}

/**
 * Extract aliases from a table reference (handles JOINs recursively)
 */
function extractAliasesFromTableRef(tableRef: any, aliases: Set<string>): void {
  if (!tableRef || typeof tableRef !== 'object') {
    return;
  }

  // Table alias
  if (tableRef.as) {
    aliases.add(tableRef.as.toLowerCase());
  }

  // Subquery alias
  if (tableRef.expr && tableRef.expr.type === 'select') {
    extractAliasesFromStatement(tableRef.expr, aliases);
  }

  // Handle JOIN clauses - node-sql-parser stores joins as array in 'join' property
  if (tableRef.join && Array.isArray(tableRef.join)) {
    for (const join of tableRef.join) {
      // Each join has a 'table' property that is itself a table reference
      if (join.table) {
        extractAliasesFromTableRef(join.table, aliases);
      }
      // Handle ON clause subqueries
      if (join.on) {
        extractAliasesFromExpression(join.on, aliases);
      }
    }
  }
}

/**
 * Extract aliases from expressions (for subqueries in WHERE, etc.)
 */
function extractAliasesFromExpression(expr: any, aliases: Set<string>): void {
  if (!expr || typeof expr !== 'object') {
    return;
  }

  // Subquery in expression
  if (expr.type === 'select') {
    extractAliasesFromStatement(expr, aliases);
  }

  // Binary expressions (recursively check left and right)
  if (expr.type === 'binary_expr') {
    if (expr.left) extractAliasesFromExpression(expr.left, aliases);
    if (expr.right) extractAliasesFromExpression(expr.right, aliases);
  }

  // IN clause with subquery
  if (expr.type === 'expr_list' && Array.isArray(expr.value)) {
    for (const val of expr.value) {
      extractAliasesFromExpression(val, aliases);
    }
  }
}

/**
 * Parse table aliases from SQL and return a map of alias -> table info
 * Handles CTEs, subqueries, regular tables, and nested JOINs
 *
 * @param sql - The SQL query text
 * @returns Map of alias (lowercase) to schema and table name
 *
 * @example
 * parseTableAliases('SELECT * FROM dbo.Users u JOIN Orders o ON u.Id = o.UserId')
 * // Returns: Map { 'u' => { schema: 'dbo', table: 'Users' }, 'o' => { schema: 'dbo', table: 'Orders' } }
 *
 * parseTableAliases('WITH cte AS (...) SELECT * FROM cte c')
 * // Returns: Map { 'c' => { schema: 'cte', table: 'cte' }, 'cte' => { schema: 'cte', table: 'cte' } }
 */
export function parseTableAliases(sql: string): Map<string, { schema: string; table: string }> {
  const aliases = new Map<string, { schema: string; table: string }>();

  if (!sql || !sql.trim()) {
    return aliases;
  }

  const ast = parseSQL(sql);

  if (!ast) {
    // Fallback: use regex (original implementation)
    const skipWords = new Set(['WHERE', 'ON', 'JOIN', 'LEFT', 'RIGHT', 'INNER', 'OUTER', 'CROSS', 'ORDER', 'GROUP', 'HAVING', 'UNION', 'SET', 'AND', 'OR', 'AS', 'SELECT', 'FROM', 'INTO', 'VALUES', 'UPDATE', 'DELETE', 'INSERT']);

    // Pattern 1: [schema].[table] AS [alias] or [schema].[table] [alias]
    const pattern1 = /(?:FROM|JOIN|,)\s+\[?(\w+)\]?\.\[?(\w+)\]?\s+(?:AS\s+)?\[?(\w+)\]?(?=\s|$|,|\)|\r|\n)/gi;
    for (const match of sql.matchAll(pattern1)) {
      const schema = match[1];
      const table = match[2];
      const alias = match[3];

      if (alias && !skipWords.has(alias.toUpperCase())) {
        aliases.set(alias.toLowerCase(), { schema, table });
      }
    }

    // Pattern 2: [table] AS [alias] or [table] [alias] (without schema)
    const pattern2 = /(?:FROM|JOIN|,)\s+(?!\[?\w+\]?\.)\[?(\w+)\]?\s+(?:AS\s+)?\[?(\w+)\]?(?=\s+(?:ON|WHERE|JOIN|LEFT|RIGHT|INNER|OUTER|CROSS|ORDER|GROUP|HAVING|UNION|,|\)|$|\r|\n))/gi;
    for (const match of sql.matchAll(pattern2)) {
      const table = match[1];
      const alias = match[2];

      if (alias && !skipWords.has(alias.toUpperCase()) && table.toUpperCase() !== alias.toUpperCase()) {
        aliases.set(alias.toLowerCase(), { schema: 'dbo', table });
      }
    }

    return aliases;
  }

  // Handle multiple statements
  const statements = Array.isArray(ast) ? ast : [ast];

  for (const stmt of statements) {
    extractAliasMapFromStatement(stmt, aliases);
  }

  return aliases;
}

/**
 * Recursively extract alias mappings from an AST statement
 */
function extractAliasMapFromStatement(
  stmt: any,
  aliases: Map<string, { schema: string; table: string }>
): void {
  if (!stmt || typeof stmt !== 'object') {
    return;
  }

  // Extract CTE aliases (WITH clause) - CTEs are tables themselves
  if (stmt.with && Array.isArray(stmt.with)) {
    for (const cte of stmt.with) {
      if (cte.name) {
        // CTE name might be a string or an object
        const cteName = typeof cte.name === 'string' ? cte.name : (cte.name.value || cte.name);
        if (typeof cteName === 'string') {
          // CTE name can be used as both table and alias
          aliases.set(cteName.toLowerCase(), { schema: 'cte', table: cteName });
        }
      }
      // Also extract aliases from CTE definition
      if (cte.stmt) {
        extractAliasMapFromStatement(cte.stmt, aliases);
      }
    }
  }

  // Extract table aliases from FROM clause
  if (stmt.from && Array.isArray(stmt.from)) {
    for (const tableRef of stmt.from) {
      extractAliasMapFromTableRef(tableRef, aliases);
    }
  }

  // Extract aliases from UPDATE/DELETE table references
  if (stmt.table && Array.isArray(stmt.table)) {
    for (const tableRef of stmt.table) {
      extractAliasMapFromTableRef(tableRef, aliases);
    }
  }

  // Extract aliases from subqueries in SELECT columns
  if (stmt.columns && Array.isArray(stmt.columns)) {
    for (const col of stmt.columns) {
      if (col.expr && col.expr.type === 'select') {
        extractAliasMapFromStatement(col.expr, aliases);
      }
    }
  }

  // Extract aliases from WHERE clause subqueries
  if (stmt.where) {
    extractAliasMapFromExpression(stmt.where, aliases);
  }
}

/**
 * Extract alias mappings from a table reference (handles JOINs recursively)
 */
function extractAliasMapFromTableRef(
  tableRef: any,
  aliases: Map<string, { schema: string; table: string }>
): void {
  if (!tableRef || typeof tableRef !== 'object') {
    return;
  }

  // Regular table with alias
  if (tableRef.table && typeof tableRef.table === 'string' && tableRef.as) {
    aliases.set(tableRef.as.toLowerCase(), {
      schema: tableRef.db || tableRef.schema || 'dbo',
      table: tableRef.table
    });
  }

  // Subquery with alias (derived table)
  if (tableRef.expr && tableRef.expr.type === 'select') {
    if (tableRef.as) {
      // The subquery itself is aliased - we can't determine its "table" but we record it
      aliases.set(tableRef.as.toLowerCase(), {
        schema: 'subquery',
        table: tableRef.as
      });
    }
    // Also extract aliases from within subquery
    extractAliasMapFromStatement(tableRef.expr, aliases);
  }

  // Handle JOIN clauses - node-sql-parser stores joins as array in 'join' property
  if (tableRef.join && Array.isArray(tableRef.join)) {
    for (const join of tableRef.join) {
      // Each join has a 'table' property that is itself a table reference
      if (join.table) {
        extractAliasMapFromTableRef(join.table, aliases);
      }
      // Handle ON clause subqueries
      if (join.on) {
        extractAliasMapFromExpression(join.on, aliases);
      }
    }
  }
}

/**
 * Extract alias mappings from expressions (for subqueries in WHERE, etc.)
 */
function extractAliasMapFromExpression(
  expr: any,
  aliases: Map<string, { schema: string; table: string }>
): void {
  if (!expr || typeof expr !== 'object') {
    return;
  }

  // Subquery in expression
  if (expr.type === 'select') {
    extractAliasMapFromStatement(expr, aliases);
  }

  // Binary expressions (recursively check left and right)
  if (expr.type === 'binary_expr') {
    if (expr.left) extractAliasMapFromExpression(expr.left, aliases);
    if (expr.right) extractAliasMapFromExpression(expr.right, aliases);
  }

  // IN clause with subquery
  if (expr.type === 'expr_list' && Array.isArray(expr.value)) {
    for (const val of expr.value) {
      extractAliasMapFromExpression(val, aliases);
    }
  }
}

/**
 * Extract table name from UPDATE statement
 *
 * @param sql - The UPDATE statement
 * @returns Table schema and name, or null if not found
 *
 * @example
 * extractUpdateTableName('UPDATE dbo.Users SET Name = "John"')
 * // Returns: { schema: 'dbo', table: 'Users' }
 *
 * extractUpdateTableName('UPDATE TOP(10) Users SET ...')
 * // Returns: { schema: 'dbo', table: 'Users' }
 */
export function extractUpdateTableName(sql: string): { schema: string; table: string } | null {
  if (!sql || !sql.trim()) {
    return null;
  }

  const ast = parseSQL(sql);

  if (!ast) {
    // Fallback: use regex
    const pattern = /\bUPDATE\s+(?:TOP\s*\(\s*\d+\s*\)\s+)?(?:\[?(\w+)\]?\.)?\[?(\w+)\]?\s+SET/i;
    const match = sql.match(pattern);
    if (match) {
      return {
        schema: match[1] || 'dbo',
        table: match[2]
      };
    }
    return null;
  }

  // Handle multiple statements (get first UPDATE)
  const statements = Array.isArray(ast) ? ast : [ast];

  for (const stmt of statements) {
    if (stmt.type === 'update' && stmt.table && Array.isArray(stmt.table) && stmt.table[0]) {
      const tableRef = stmt.table[0];
      return {
        schema: tableRef.db || tableRef.schema || 'dbo',
        table: tableRef.table
      };
    }
  }

  return null;
}

/**
 * Extract table name from INSERT statement
 *
 * @param sql - The INSERT statement
 * @returns Table schema and name, or null if not found
 *
 * @example
 * extractInsertTableName('INSERT INTO dbo.Users (Name) VALUES ("John")')
 * // Returns: { schema: 'dbo', table: 'Users' }
 *
 * extractInsertTableName('INSERT Users VALUES (...)')
 * // Returns: { schema: 'dbo', table: 'Users' }
 */
export function extractInsertTableName(sql: string): { schema: string; table: string } | null {
  if (!sql || !sql.trim()) {
    return null;
  }

  const ast = parseSQL(sql);

  if (!ast) {
    // Fallback: use regex
    const pattern = /\bINSERT\s+(?:INTO\s+)?(?:\[?(\w+)\]?\.)?\[?(\w+)\]?/i;
    const match = sql.match(pattern);
    if (match) {
      return {
        schema: match[1] || 'dbo',
        table: match[2]
      };
    }
    return null;
  }

  // Handle multiple statements (get first INSERT)
  const statements = Array.isArray(ast) ? ast : [ast];

  for (const stmt of statements) {
    if (stmt.type === 'insert' && stmt.table && Array.isArray(stmt.table) && stmt.table[0]) {
      const tableRef = stmt.table[0];
      return {
        schema: tableRef.db || tableRef.schema || 'dbo',
        table: tableRef.table
      };
    }
  }

  return null;
}

/**
 * Extract all table references from a SQL query (FROM, JOIN, subqueries, CTEs)
 *
 * @param sql - The SQL query text
 * @returns Array of table references with schema and table name
 *
 * @example
 * extractReferencedTables('SELECT * FROM dbo.Users u JOIN Orders o ON u.Id = o.UserId')
 * // Returns: [{ schema: 'dbo', table: 'Users' }, { schema: 'dbo', table: 'Orders' }]
 *
 * extractReferencedTables('WITH cte AS (SELECT * FROM Products) SELECT * FROM cte')
 * // Returns: [{ schema: 'dbo', table: 'Products' }]
 */
export function extractReferencedTables(sql: string): Array<{ schema: string; table: string }> {
  const tables: Array<{ schema: string; table: string }> = [];

  if (!sql || !sql.trim()) {
    return tables;
  }

  const ast = parseSQL(sql);

  if (!ast) {
    // Fallback: use regex
    const pattern = /(?:FROM|JOIN|INTO|UPDATE)\s+\[?(\w+)\]?(?:\.\[?(\w+)\]?)?/gi;
    let match;
    while ((match = pattern.exec(sql)) !== null) {
      if (match[2]) {
        tables.push({ schema: match[1], table: match[2] });
      } else {
        tables.push({ schema: 'dbo', table: match[1] });
      }
    }
    return tables;
  }

  // Handle multiple statements
  const statements = Array.isArray(ast) ? ast : [ast];

  for (const stmt of statements) {
    extractTablesFromStatement(stmt, tables);
  }

  return tables;
}

/**
 * Recursively extract table references from an AST statement
 */
function extractTablesFromStatement(
  stmt: any,
  tables: Array<{ schema: string; table: string }>
): void {
  if (!stmt || typeof stmt !== 'object') {
    return;
  }

  // Extract tables from CTEs (WITH clause) - recursively extract from CTE definitions
  if (stmt.with && Array.isArray(stmt.with)) {
    for (const cte of stmt.with) {
      if (cte.stmt) {
        extractTablesFromStatement(cte.stmt, tables);
      }
    }
  }

  // Extract tables from FROM clause
  if (stmt.from && Array.isArray(stmt.from)) {
    for (const tableRef of stmt.from) {
      extractTablesFromTableRef(tableRef, tables);
    }
  }

  // Extract tables from UPDATE/DELETE/INSERT
  if (stmt.table && Array.isArray(stmt.table)) {
    for (const tableRef of stmt.table) {
      if (tableRef.table) {
        tables.push({
          schema: tableRef.db || tableRef.schema || 'dbo',
          table: tableRef.table
        });
      }
    }
  }

  // Extract tables from subqueries in SELECT columns
  if (stmt.columns && Array.isArray(stmt.columns)) {
    for (const col of stmt.columns) {
      if (col.expr && col.expr.type === 'select') {
        extractTablesFromStatement(col.expr, tables);
      }
    }
  }

  // Extract tables from WHERE clause subqueries
  if (stmt.where) {
    extractTablesFromExpression(stmt.where, tables);
  }
}

/**
 * Extract tables from a table reference (handles JOINs recursively)
 */
function extractTablesFromTableRef(
  tableRef: any,
  tables: Array<{ schema: string; table: string }>
): void {
  if (!tableRef || typeof tableRef !== 'object') {
    return;
  }

  // Regular table reference
  if (tableRef.table && typeof tableRef.table === 'string') {
    tables.push({
      schema: tableRef.db || tableRef.schema || 'dbo',
      table: tableRef.table
    });
  }

  // Subquery (derived table)
  if (tableRef.expr && tableRef.expr.type === 'select') {
    extractTablesFromStatement(tableRef.expr, tables);
  }

  // Handle JOIN clauses - node-sql-parser stores joins as array in 'join' property
  if (tableRef.join && Array.isArray(tableRef.join)) {
    for (const join of tableRef.join) {
      // Each join has a 'table' property that is itself a table reference
      if (join.table) {
        extractTablesFromTableRef(join.table, tables);
      }
      // Handle ON clause subqueries
      if (join.on) {
        extractTablesFromExpression(join.on, tables);
      }
    }
  }
}

/**
 * Extract tables from expressions (for subqueries in WHERE, etc.)
 */
function extractTablesFromExpression(
  expr: any,
  tables: Array<{ schema: string; table: string }>
): void {
  if (!expr || typeof expr !== 'object') {
    return;
  }

  // Subquery in expression
  if (expr.type === 'select') {
    extractTablesFromStatement(expr, tables);
  }

  // Binary expressions (recursively check left and right)
  if (expr.type === 'binary_expr') {
    if (expr.left) extractTablesFromExpression(expr.left, tables);
    if (expr.right) extractTablesFromExpression(expr.right, tables);
  }

  // IN clause with subquery
  if (expr.type === 'expr_list' && Array.isArray(expr.value)) {
    for (const val of expr.value) {
      extractTablesFromExpression(val, tables);
    }
  }
}

/**
 * Get completion context at cursor position using hybrid AST + regex approach
 * This is optimized for performance (called on every keystroke) while providing
 * better accuracy than pure regex for complex SQL.
 *
 * @param textBeforeCursor - Text from start of document to cursor position
 * @param fullText - Full document text (optional, for better AST analysis)
 * @returns Completion context information
 *
 * @example
 * getCompletionContext('SELECT * FROM Users u WHERE u.')
 * // Returns: { type: 'alias_column', alias: 'u' }
 *
 * getCompletionContext('UPDATE Users SET ')
 * // Returns: { type: 'update_column', targetTable: { schema: 'dbo', table: 'Users' } }
 */
export function getCompletionContext(
  textBeforeCursor: string,
  fullText?: string
): CompletionContext {
  const trimmed = textBeforeCursor.trim().toUpperCase();

  // Quick check for alias.column pattern (common case - keep fast)
  const aliasMatch = textBeforeCursor.match(/\[?(\w+)\]?\.\s*$/);
  if (aliasMatch) {
    const potentialAlias = aliasMatch[1].toLowerCase();
    // Check if it's a known schema prefix
    if (['dbo', 'sys', 'information_schema'].includes(potentialAlias)) {
      return { type: 'schema', partialWord: potentialAlias };
    }
    return { type: 'alias_column', alias: potentialAlias };
  }

  // IMPORTANT: Check for table-context keywords BEFORE AST analysis
  // Keywords like FROM, JOIN are unambiguous and keyword detection is faster/more reliable
  const words = trimmed.split(/\s+/);
  const lastWord = words[words.length - 1];
  const secondLastWord = words.length > 1 ? words[words.length - 2] : '';
  const combinedKeyword = `${secondLastWord} ${lastWord}`;

  const TABLE_CONTEXT_KEYWORDS = ['FROM', 'JOIN', 'INNER JOIN', 'LEFT JOIN', 'RIGHT JOIN', 'OUTER JOIN', 'CROSS JOIN', 'INTO', 'UPDATE', 'TABLE', 'APPLY', 'CROSS APPLY', 'OUTER APPLY'];

  // Prioritize table context keywords - these are unambiguous
  if (TABLE_CONTEXT_KEYWORDS.includes(combinedKeyword)) {
    return { type: 'table', lastKeyword: combinedKeyword };
  }

  if (TABLE_CONTEXT_KEYWORDS.includes(lastWord)) {
    return { type: 'table', lastKeyword: lastWord };
  }

  // Try AST-based analysis if full text is provided (more accurate but slower)
  if (fullText && fullText.trim().length > 0 && fullText.trim().length < 10000) {
    // Only use AST for reasonably sized queries to avoid performance issues
    const astContext = getCompletionContextFromAST(textBeforeCursor, fullText);
    if (astContext) {
      return astContext;
    }
  }

  // Fallback to keyword-based detection (fast, works for most cases)

  // UPDATE context detection
  if (lastWord === 'SET' && /\bUPDATE\b/i.test(textBeforeCursor)) {
    const updateTable = extractUpdateTableName(textBeforeCursor);
    if (updateTable) {
      return { type: 'update_column', targetTable: updateTable, statementType: 'update' };
    }
  }

  if (/\bUPDATE\b.*\bSET\b.*,\s*$/i.test(textBeforeCursor)) {
    const updateTable = extractUpdateTableName(textBeforeCursor);
    if (updateTable) {
      return { type: 'update_column', targetTable: updateTable, statementType: 'update' };
    }
  }

  if (/\bUPDATE\b.*\bWHERE\b/i.test(textBeforeCursor)) {
    const updateTable = extractUpdateTableName(textBeforeCursor);
    if (updateTable) {
      return { type: 'update_column', targetTable: updateTable, statementType: 'update' };
    }
  }

  // INSERT context detection
  if (/\bINSERT\s+(?:INTO\s+)?[^(]*\([^)]*$/i.test(textBeforeCursor)) {
    const insertTable = extractInsertTableName(textBeforeCursor);
    if (insertTable) {
      return { type: 'insert_column', targetTable: insertTable, statementType: 'insert' };
    }
  }

  // Keyword-based context detection
  const COLUMN_CONTEXT_KEYWORDS = ['SELECT', 'WHERE', 'ON', 'AND', 'OR', 'SET', 'ORDER BY', 'GROUP BY', 'HAVING', 'BY', '='];
  const ROUTINE_CONTEXT_KEYWORDS = ['EXEC', 'EXECUTE', 'CALL'];
  const DATABASE_CONTEXT_KEYWORDS = ['USE'];

  if (DATABASE_CONTEXT_KEYWORDS.includes(lastWord)) {
    return { type: 'database', lastKeyword: lastWord };
  }

  if (ROUTINE_CONTEXT_KEYWORDS.includes(lastWord)) {
    return { type: 'routine', lastKeyword: lastWord };
  }

  if (COLUMN_CONTEXT_KEYWORDS.includes(lastWord) || COLUMN_CONTEXT_KEYWORDS.includes(combinedKeyword)) {
    return { type: 'column', lastKeyword: lastWord };
  }

  // Check for comparison operators
  if (/[=<>!]+$/.test(trimmed) || /\s+(LIKE|IN|BETWEEN)\s*$/i.test(trimmed)) {
    return { type: 'column' };
  }

  // Inside function call
  if (/\(\s*$/.test(textBeforeCursor) || /,\s*$/.test(textBeforeCursor)) {
    return { type: 'column' };
  }

  // Default to keyword
  return { type: 'keyword' };
}

/**
 * Get completion context using AST analysis (more accurate but slower)
 * Returns null if AST parsing fails or context can't be determined
 */
function getCompletionContextFromAST(
  textBeforeCursor: string,
  fullText: string
): CompletionContext | null {
  try {
    const ast = parseSQL(fullText);
    if (!ast) {
      return null;
    }

    // Get cursor position in the full text
    const cursorOffset = textBeforeCursor.length;

    // Handle multiple statements - find which statement contains the cursor
    const statements = Array.isArray(ast) ? ast : [ast];

    for (const stmt of statements) {
      const context = analyzeStatementContext(stmt, cursorOffset, fullText);
      if (context) {
        return context;
      }
    }

    return null;
  } catch (error) {
    // AST parsing failed - return null to fallback to regex
    return null;
  }
}

/**
 * Analyze a single statement to determine completion context
 */
function analyzeStatementContext(
  stmt: any,
  _cursorOffset: number,
  _fullText: string
): CompletionContext | null {
  if (!stmt || typeof stmt !== 'object') {
    return null;
  }

  // Determine statement type
  const statementType = stmt.type as 'select' | 'insert' | 'update' | 'delete' | 'create' | 'alter' | 'drop';

  // For UPDATE statements, provide enhanced context
  if (statementType === 'update' && stmt.table && Array.isArray(stmt.table) && stmt.table[0]) {
    const tableRef = stmt.table[0];
    return {
      type: 'update_column',
      statementType: 'update',
      targetTable: {
        schema: tableRef.db || tableRef.schema || 'dbo',
        table: tableRef.table
      }
    };
  }

  // For INSERT statements
  if (statementType === 'insert' && stmt.table && Array.isArray(stmt.table) && stmt.table[0]) {
    const tableRef = stmt.table[0];
    return {
      type: 'insert_column',
      statementType: 'insert',
      targetTable: {
        schema: tableRef.db || tableRef.schema || 'dbo',
        table: tableRef.table
      }
    };
  }

  // For SELECT statements, detect if we're in a CTE
  if (statementType === 'select') {
    const isInCTE = stmt.with && Array.isArray(stmt.with) && stmt.with.length > 0;
    return {
      type: 'column',
      statementType: 'select',
      isInCTE
    };
  }

  return null;
}