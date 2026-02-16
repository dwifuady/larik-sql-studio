/**
 * SQL Validator using node-sql-parser
 * Validates SQL syntax and semantic correctness against database schema
 */

import NodeSqlParser from 'node-sql-parser';
import type { SchemaInfo, TableInfo, SchemaColumnInfo } from '../types';

const { Parser } = NodeSqlParser;

/** Validation error with location information */
export interface ValidationError {
  message: string;
  line: number;
  column: number;
  endLine: number;
  endColumn: number;
  severity: 'error' | 'warning' | 'info';
  code?: string;
}

/** Table context for column resolution */
interface TableContext {
  alias: string | null;
  schema: string;
  table: string;
  tableInfo: TableInfo | null;
}

/**
 * SQL Validator for syntax and schema validation
 */
export class SqlValidator {
  private parser: InstanceType<typeof Parser>;

  constructor() {
    this.parser = new Parser();
  }

  /**
   * Validate SQL query and return errors
   * @param query - The SQL query to validate
   * @param schemaInfo - Optional schema information for semantic validation
   * @returns Array of validation errors
   */
  validateQuery(query: string, schemaInfo: SchemaInfo | null): ValidationError[] {
    const errors: ValidationError[] = [];

    // Empty or whitespace-only queries are valid (no errors)
    if (!query || query.trim().length === 0) {
      return errors;
    }

    // Step 1: Parse syntax with appropriate database dialect
    let ast: any;
    try {
      // Try TransactSQL first for SQL Server specific syntax
      try {
        ast = this.parser.astify(query, { database: 'TransactSQL' });
      } catch (tsqlErr) {
        // Fallback to MySQL dialect for standard SQL
        ast = this.parser.astify(query, { database: 'MySQL' });
      }
    } catch (err: any) {
      // Syntax error - try to find the actual error location in the query
      const { line, column, message } = this.extractErrorInfo(err, query);
      errors.push({
        message: `Syntax error: ${message}`,
        line,
        column,
        endLine: line,
        endColumn: column + 10, // Approximate end
        severity: 'error',
        code: 'SYNTAX_ERROR',
      });
      return errors; // Can't validate semantics if syntax is broken
    }

    // Step 2: Validate tables and columns if schema available
    if (schemaInfo) {
      try {
        errors.push(...this.validateSemantics(ast, schemaInfo, query));
      } catch (err: any) {
        // Semantic validation failed - log but don't crash
        console.error('Semantic validation error:', err);
      }
    }

    return errors;
  }

  /**
   * Validate semantics (tables and columns) against schema
   */
  private validateSemantics(ast: any, schemaInfo: SchemaInfo, query: string): ValidationError[] {
    const errors: ValidationError[] = [];

    // Handle multi-statement queries (array of ASTs)
    const statements = Array.isArray(ast) ? ast : [ast];

    // Split query by semicolons to track statement positions
    const statementTexts = query.split(';').map(s => s.trim()).filter(s => s.length > 0);

    let currentLineOffset = 0;

    for (let i = 0; i < statements.length; i++) {
      const statement = statements[i];
      if (!statement || typeof statement !== 'object') {
        continue;
      }

      // Find the line offset for this statement
      if (i < statementTexts.length) {
        const statementText = statementTexts[i];
        const statementStart = query.indexOf(statementText, currentLineOffset > 0 ?
          query.split('\n').slice(0, currentLineOffset).join('\n').length : 0);

        if (statementStart !== -1) {
          const textBeforeStatement = query.substring(0, statementStart);
          currentLineOffset = (textBeforeStatement.match(/\n/g) || []).length;
        }
      }

      // Validate based on statement type with line offset
      if (statement.type === 'select') {
        errors.push(...this.validateSelectStatement(statement, schemaInfo, query, currentLineOffset));
      } else if (statement.type === 'insert') {
        errors.push(...this.validateInsertStatement(statement, schemaInfo, query, currentLineOffset));
      } else if (statement.type === 'update') {
        errors.push(...this.validateUpdateStatement(statement, schemaInfo, query, currentLineOffset));
      } else if (statement.type === 'delete') {
        errors.push(...this.validateDeleteStatement(statement, schemaInfo, query, currentLineOffset));
      }
    }

    return errors;
  }

  /**
   * Validate SELECT statement
   */
  private validateSelectStatement(
    ast: any,
    schemaInfo: SchemaInfo,
    query: string,
    lineOffset: number,
    isWithinCTE: boolean = false
  ): ValidationError[] {
    const errors: ValidationError[] = [];

    // Collect CTE names to exclude from table validation
    const cteNames = new Set<string>();
    if (ast.with && Array.isArray(ast.with)) {
      for (const cte of ast.with) {
        if (cte.name) {
          // CTE name might be a string or an object with a value property
          const cteName = typeof cte.name === 'string' ? cte.name : (cte.name.value || cte.name);
          if (typeof cteName === 'string') {
            cteNames.add(cteName.toLowerCase());
          }
        }
      }
    }

    // VALIDATE CTE DEFINITIONS FIRST
    // Each CTE is a SELECT statement that needs validation
    if (ast.with && Array.isArray(ast.with)) {
      for (const cte of ast.with) {
        if (cte.stmt && cte.stmt.type === 'select') {
          // Recursively validate the CTE's SELECT statement
          // Mark that we're within a CTE to skip SELECT * warnings
          errors.push(...this.validateSelectStatement(cte.stmt, schemaInfo, query, lineOffset, true));
        }
      }
    }

    // Build table context from FROM clause (includes CTEs if present)
    const tableContext = this.buildTableContext(ast.from, schemaInfo);

    // Add CTEs to table context
    if (ast.with && Array.isArray(ast.with)) {
      for (const cte of ast.with) {
        if (cte.name) {
          // CTE name might be a string or an object
          const cteName = typeof cte.name === 'string' ? cte.name : (cte.name.value || cte.name);
          if (typeof cteName === 'string') {
            tableContext.push({
              alias: null,
              schema: 'cte',
              table: cteName,
              tableInfo: null, // CTEs don't have schema info
            });
          }
        }
      }
    }

    // Validate table references in FROM clause (skip CTEs)
    errors.push(...this.validateTableReferences(ast.from, schemaInfo, query, lineOffset, cteNames));

    // Warning: SELECT * usage (skip for CTEs - they're intermediate results)
    if (!isWithinCTE && ast.columns && Array.isArray(ast.columns)) {
      const hasSelectStar = ast.columns.some(
        (col: any) => col.expr && col.expr.type === 'column_ref' && col.expr.column === '*'
      );
      if (hasSelectStar && ast.from && ast.from.length > 0) {
        const selectPos = this.findIdentifierPosition(query, 'SELECT', lineOffset);
        errors.push({
          message: 'Consider specifying columns explicitly instead of using SELECT *',
          line: selectPos.line,
          column: selectPos.column,
          endLine: selectPos.line,
          endColumn: selectPos.column + 8, // length of "SELECT *"
          severity: 'info',
          code: 'SELECT_STAR',
        });
      }

      // Validate column references in SELECT list
      for (const column of ast.columns) {
        if (column.expr && column.expr.type === 'column_ref') {
          errors.push(...this.validateColumnReference(column.expr, tableContext, query, lineOffset));
        }
      }
    }

    // Validate column references in WHERE clause
    if (ast.where) {
      errors.push(...this.validateExpression(ast.where, tableContext, schemaInfo, query, lineOffset));
    }

    // Validate column references in ORDER BY
    if (ast.orderby && Array.isArray(ast.orderby)) {
      for (const orderItem of ast.orderby) {
        if (orderItem.expr && orderItem.expr.type === 'column_ref') {
          errors.push(...this.validateColumnReference(orderItem.expr, tableContext, query, lineOffset));
        }
      }
    }

    // Validate column references in GROUP BY
    if (ast.groupby && Array.isArray(ast.groupby)) {
      for (const groupItem of ast.groupby) {
        if (groupItem.type === 'column_ref') {
          errors.push(...this.validateColumnReference(groupItem, tableContext, query, lineOffset));
        }
      }
    }

    return errors;
  }

  /**
   * Validate INSERT statement
   */
  private validateInsertStatement(ast: any, schemaInfo: SchemaInfo, query: string, lineOffset: number): ValidationError[] {
    const errors: ValidationError[] = [];

    // Validate target table
    if (ast.table && Array.isArray(ast.table)) {
      errors.push(...this.validateTableReferences(ast.table, schemaInfo, query, lineOffset));

      // Validate column list if present
      if (ast.columns && Array.isArray(ast.columns) && ast.table[0]) {
        const tableRef = ast.table[0];
        const table = this.findTable(
          schemaInfo,
          tableRef.db || tableRef.schema || 'dbo',
          tableRef.table
        );

        if (table) {
          for (const columnName of ast.columns) {
            const column = this.findColumn(table, columnName);
            if (!column) {
              errors.push({
                message: `Column '${columnName}' does not exist in table '${table.schema_name}.${table.table_name}'`,
                line: 1,
                column: 1,
                endLine: 1,
                endColumn: 1 + columnName.length,
                severity: 'error',
                code: 'COLUMN_NOT_FOUND',
              });
            }
          }
        }
      }
    }

    return errors;
  }

  /**
   * Validate UPDATE statement
   */
  private validateUpdateStatement(ast: any, schemaInfo: SchemaInfo, query: string, lineOffset: number): ValidationError[] {
    const errors: ValidationError[] = [];

    // Validate target table
    if (ast.table && Array.isArray(ast.table)) {
      errors.push(...this.validateTableReferences(ast.table, schemaInfo, query, lineOffset));

      const tableContext = this.buildTableContext(ast.table, schemaInfo);

      // Validate SET clause columns
      if (ast.set && Array.isArray(ast.set)) {
        for (const setItem of ast.set) {
          if (setItem.column) {
            const columnRef = {
              type: 'column_ref',
              table: setItem.table || null,
              column: setItem.column,
            };
            errors.push(...this.validateColumnReference(columnRef, tableContext, query, lineOffset));
          }
        }
      }

      // Validate WHERE clause
      if (ast.where) {
        errors.push(...this.validateExpression(ast.where, tableContext, schemaInfo, query, lineOffset));
      } else {
        // Warning: UPDATE without WHERE
        const updatePos = this.findIdentifierPosition(query, 'UPDATE', lineOffset);
        errors.push({
          message: 'UPDATE statement without WHERE clause will affect all rows in the table',
          line: updatePos.line,
          column: updatePos.column,
          endLine: updatePos.line,
          endColumn: updatePos.column + 6, // length of "UPDATE"
          severity: 'warning',
          code: 'UPDATE_WITHOUT_WHERE',
        });
      }
    }

    return errors;
  }

  /**
   * Validate DELETE statement
   */
  private validateDeleteStatement(ast: any, schemaInfo: SchemaInfo, query: string, lineOffset: number): ValidationError[] {
    const errors: ValidationError[] = [];

    // Validate target table
    if (ast.from && Array.isArray(ast.from)) {
      errors.push(...this.validateTableReferences(ast.from, schemaInfo, query, lineOffset));

      const tableContext = this.buildTableContext(ast.from, schemaInfo);

      // Validate WHERE clause
      if (ast.where) {
        errors.push(...this.validateExpression(ast.where, tableContext, schemaInfo, query, lineOffset));
      } else {
        // Warning: DELETE without WHERE
        const deletePos = this.findIdentifierPosition(query, 'DELETE', lineOffset);
        errors.push({
          message: 'DELETE statement without WHERE clause will delete all rows in the table',
          line: deletePos.line,
          column: deletePos.column,
          endLine: deletePos.line,
          endColumn: deletePos.column + 6, // length of "DELETE"
          severity: 'warning',
          code: 'DELETE_WITHOUT_WHERE',
        });
      }
    }

    return errors;
  }

  /**
   * Build table context from FROM clause for column resolution
   */
  private buildTableContext(fromClause: any[] | null, schemaInfo: SchemaInfo): TableContext[] {
    const context: TableContext[] = [];

    if (!fromClause || !Array.isArray(fromClause)) {
      return context;
    }

    for (const tableRef of fromClause) {
      if (!tableRef || typeof tableRef !== 'object') {
        continue;
      }

      const schema = tableRef.db || tableRef.schema || 'dbo';
      const tableName = tableRef.table;
      const alias = tableRef.as || null;

      const tableInfo = this.findTable(schemaInfo, schema, tableName);

      context.push({
        alias,
        schema,
        table: tableName,
        tableInfo,
      });
    }

    return context;
  }

  /**
   * Validate table references in FROM/JOIN clauses
   */
  private validateTableReferences(
    fromClause: any[] | null,
    schemaInfo: SchemaInfo,
    query: string,
    lineOffset: number,
    cteNames: Set<string> = new Set()
  ): ValidationError[] {
    const errors: ValidationError[] = [];

    if (!fromClause || !Array.isArray(fromClause)) {
      return errors;
    }

    for (const tableRef of fromClause) {
      if (!tableRef || typeof tableRef !== 'object' || !tableRef.table) {
        continue;
      }

      const schema = tableRef.db || tableRef.schema || 'dbo';
      const tableName = tableRef.table;

      // Skip validation for CTEs - they're not real tables in the schema
      if (cteNames.has(tableName.toLowerCase())) {
        // CTE reference is valid, skip schema validation
        if (tableRef.on) {
          const tableContext = this.buildTableContext(fromClause, schemaInfo);
          errors.push(...this.validateExpression(tableRef.on, tableContext, schemaInfo, query, lineOffset));
        }
        continue;
      }

      const table = this.findTable(schemaInfo, schema, tableName);

      if (!table) {
        const suggestion = this.suggestTable(schemaInfo, tableName);
        const suggestionText = suggestion ? ` Did you mean '${suggestion.schema_name}.${suggestion.table_name}'?` : '';

        // Find the actual position of the table name in the query
        const position = this.findIdentifierPosition(query, tableName, lineOffset);

        errors.push({
          message: `Table '${schema}.${tableName}' does not exist in database '${schemaInfo.database_name}'.${suggestionText}`,
          line: position.line,
          column: position.column,
          endLine: position.line,
          endColumn: position.column + tableName.length,
          severity: 'error',
          code: 'TABLE_NOT_FOUND',
        });
      }

      // Validate JOIN ON conditions
      if (tableRef.on) {
        const tableContext = this.buildTableContext(fromClause, schemaInfo);
        errors.push(...this.validateExpression(tableRef.on, tableContext, schemaInfo, query, lineOffset));
      }
    }

    return errors;
  }

  /**
   * Validate a column reference
   */
  private validateColumnReference(
    columnRef: any,
    tableContext: TableContext[],
    query: string,
    lineOffset: number
  ): ValidationError[] {
    const errors: ValidationError[] = [];

    if (!columnRef || columnRef.type !== 'column_ref') {
      return errors;
    }

    const columnName = columnRef.column;
    const tableQualifier = columnRef.table;

    // Skip special cases
    if (columnName === '*' || columnName === 'expr') {
      return errors;
    }

    // If table is qualified, validate against that specific table
    if (tableQualifier) {
      const context = tableContext.find(
        (ctx) => ctx.alias === tableQualifier || ctx.table.toLowerCase() === tableQualifier.toLowerCase()
      );

      if (!context || !context.tableInfo) {
        // Table qualifier not found or table doesn't exist (already reported)
        return errors;
      }

      const column = this.findColumn(context.tableInfo, columnName);
      if (!column) {
        const suggestion = this.suggestColumn(context.tableInfo, columnName);
        const suggestionText = suggestion ? ` Did you mean '${suggestion.name}'?` : '';

        // Find the actual position of the column name in the query
        const position = this.findIdentifierPosition(query, columnName, lineOffset);

        errors.push({
          message: `Column '${columnName}' does not exist in table '${context.tableInfo.schema_name}.${context.tableInfo.table_name}'.${suggestionText}`,
          line: position.line,
          column: position.column,
          endLine: position.line,
          endColumn: position.column + columnName.length,
          severity: 'error',
          code: 'COLUMN_NOT_FOUND',
        });
      }
    } else {
      // Unqualified column - search in all tables in context
      const matchingTables: TableContext[] = [];

      for (const context of tableContext) {
        if (context.tableInfo && this.findColumn(context.tableInfo, columnName)) {
          matchingTables.push(context);
        }
      }

      if (matchingTables.length === 0) {
        // Column not found in any table
        const position = this.findIdentifierPosition(query, columnName, lineOffset);

        errors.push({
          message: `Column '${columnName}' not found in any table in the query`,
          line: position.line,
          column: position.column,
          endLine: position.line,
          endColumn: position.column + columnName.length,
          severity: 'error',
          code: 'COLUMN_NOT_FOUND',
        });
      } else if (matchingTables.length > 1) {
        // Ambiguous column
        const position = this.findIdentifierPosition(query, columnName, lineOffset);
        const tableNames = matchingTables.map((ctx) => `${ctx.schema}.${ctx.table}`).join(', ');

        errors.push({
          message: `Column '${columnName}' is ambiguous. It exists in multiple tables: ${tableNames}. Use table alias or qualified name.`,
          line: position.line,
          column: position.column,
          endLine: position.line,
          endColumn: position.column + columnName.length,
          severity: 'warning',
          code: 'AMBIGUOUS_COLUMN',
        });
      }
    }

    return errors;
  }

  /**
   * Validate an expression (recursively walks expression tree)
   */
  private validateExpression(
    expr: any,
    tableContext: TableContext[],
    schemaInfo: SchemaInfo,
    query: string,
    lineOffset: number
  ): ValidationError[] {
    const errors: ValidationError[] = [];

    if (!expr || typeof expr !== 'object') {
      return errors;
    }

    // Validate column references
    if (expr.type === 'column_ref') {
      errors.push(...this.validateColumnReference(expr, tableContext, query, lineOffset));
    }

    // Recursively validate binary expressions (AND, OR, =, >, <, etc.)
    if (expr.type === 'binary_expr') {
      if (expr.left) {
        errors.push(...this.validateExpression(expr.left, tableContext, schemaInfo, query, lineOffset));
      }
      if (expr.right) {
        errors.push(...this.validateExpression(expr.right, tableContext, schemaInfo, query, lineOffset));
      }
    }

    // Recursively validate unary expressions (NOT, etc.)
    if (expr.type === 'unary_expr' && expr.expr) {
      errors.push(...this.validateExpression(expr.expr, tableContext, schemaInfo, query, lineOffset));
    }

    // Validate function arguments
    if (expr.type === 'aggr_func' || expr.type === 'function') {
      if (expr.args && Array.isArray(expr.args.expr)) {
        for (const arg of expr.args.expr) {
          errors.push(...this.validateExpression(arg, tableContext, schemaInfo, query, lineOffset));
        }
      }
    }

    return errors;
  }

  /**
   * Find a table in schema (case-insensitive)
   */
  private findTable(schemaInfo: SchemaInfo, schemaName: string, tableName: string): TableInfo | null {
    return (
      schemaInfo.tables.find(
        (t) =>
          t.table_name.toLowerCase() === tableName.toLowerCase() &&
          t.schema_name.toLowerCase() === schemaName.toLowerCase()
      ) || null
    );
  }

  /**
   * Find a column in table (case-insensitive)
   */
  private findColumn(table: TableInfo, columnName: string): SchemaColumnInfo | null {
    return table.columns.find((c) => c.name.toLowerCase() === columnName.toLowerCase()) || null;
  }

  /**
   * Suggest a similar table name using simple string matching
   */
  private suggestTable(schemaInfo: SchemaInfo, tableName: string): TableInfo | null {
    const lower = tableName.toLowerCase();

    // Try to find table with similar name
    const similar = schemaInfo.tables.find((t) => {
      const tableNameLower = t.table_name.toLowerCase();
      // Check if it's a simple typo (one character difference)
      return this.levenshteinDistance(tableNameLower, lower) <= 2;
    });

    return similar || null;
  }

  /**
   * Suggest a similar column name
   */
  private suggestColumn(table: TableInfo, columnName: string): SchemaColumnInfo | null {
    const lower = columnName.toLowerCase();

    const similar = table.columns.find((c) => {
      const colNameLower = c.name.toLowerCase();
      return this.levenshteinDistance(colNameLower, lower) <= 2;
    });

    return similar || null;
  }

  /**
   * Calculate Levenshtein distance for typo suggestions
   */
  private levenshteinDistance(str1: string, str2: string): number {
    const m = str1.length;
    const n = str2.length;
    const dp: number[][] = [];

    for (let i = 0; i <= m; i++) {
      dp[i] = [i];
    }

    for (let j = 0; j <= n; j++) {
      dp[0][j] = j;
    }

    for (let i = 1; i <= m; i++) {
      for (let j = 1; j <= n; j++) {
        if (str1[i - 1] === str2[j - 1]) {
          dp[i][j] = dp[i - 1][j - 1];
        } else {
          dp[i][j] = Math.min(dp[i - 1][j - 1], dp[i - 1][j], dp[i][j - 1]) + 1;
        }
      }
    }

    return dp[m][n];
  }

  /**
   * Extract error information from parser exception
   */
  private extractErrorInfo(err: any, _query: string): { line: number; column: number; message: string } {
    let line = 1;
    let column = 1;
    let message = err.message || 'Unknown syntax error';

    // Try to extract line/column from error message
    // Common formats: "line 3 column 15", "at line 3, column 15"
    const lineMatch = message.match(/line\s+(\d+)/i);
    const columnMatch = message.match(/column\s+(\d+)/i);

    if (lineMatch) {
      line = parseInt(lineMatch[1], 10);
    }
    if (columnMatch) {
      column = parseInt(columnMatch[1], 10);
    }

    // Check if error object has location property
    if (err.location) {
      if (err.location.start) {
        line = err.location.start.line || line;
        column = err.location.start.column || column;
      }
    }

    return { line, column, message };
  }

  /**
   * Find the position of an identifier (table/column name) in the query text
   */
  private findIdentifierPosition(
    query: string,
    identifier: string,
    lineOffset: number,
    _context?: string
  ): { line: number; column: number } {
    const lines = query.split('\n');

    // Escape special regex characters in the identifier
    const escapedIdentifier = identifier.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

    // Search from lineOffset onwards
    for (let i = lineOffset; i < lines.length; i++) {
      const line = lines[i];

      // Try different patterns and find the actual position of the identifier
      // Pattern 1: [identifier] in brackets
      let match = line.match(new RegExp(`\\[${escapedIdentifier}\\]`, 'i'));
      if (match && match.index !== undefined) {
        return {
          line: i + 1,
          column: match.index + 2, // +2 to skip the opening bracket and point to identifier
        };
      }

      // Pattern 2: alias.[identifier]
      match = line.match(new RegExp(`\\[?\\w+\\]?\\.\\[${escapedIdentifier}\\]`, 'i'));
      if (match && match.index !== undefined) {
        // Find where [identifier] starts within the match
        const identifierStart = match[0].indexOf('[', 1); // Find second bracket
        if (identifierStart !== -1) {
          return {
            line: i + 1,
            column: match.index + identifierStart + 2, // +2 to skip opening bracket
          };
        }
      }

      // Pattern 3: alias.identifier (no brackets)
      match = line.match(new RegExp(`\\w+\\.${escapedIdentifier}\\b`, 'i'));
      if (match && match.index !== undefined) {
        // Find where identifier starts within the match
        const dotPos = match[0].indexOf('.');
        if (dotPos !== -1) {
          return {
            line: i + 1,
            column: match.index + dotPos + 2, // +2 to skip dot and point to identifier
          };
        }
      }

      // Pattern 4: identifier without prefix
      match = line.match(new RegExp(`\\b${escapedIdentifier}\\b`, 'i'));
      if (match && match.index !== undefined) {
        return {
          line: i + 1,
          column: match.index + 1,
        };
      }
    }

    // Fallback: return the line offset + 1
    return { line: lineOffset + 1, column: 1 };
  }
}
