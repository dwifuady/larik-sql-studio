/**
 * SQL Formatter utility using sql-formatter library
 * Configured for SQL Server (T-SQL) with professional formatting
 */

import { format } from 'sql-formatter';

/**
 * Format SQL with proper indentation for readability
 * Uses sql-formatter library with T-SQL dialect
 */
export function formatSqlWithIndentation(sql: string): string {
  if (!sql || !sql.trim()) return sql;

  try {
    return format(sql, {
      language: 'tsql', // SQL Server / T-SQL
      tabWidth: 2,
      keywordCase: 'upper',
      indentStyle: 'standard',
      linesBetweenQueries: 2,
      denseOperators: false,
      newlineBeforeSemicolon: false,
    });
  } catch (error) {
    // If formatting fails, return original SQL
    console.error('SQL formatting error:', error);
    return sql;
  }
}

/**
 * Format SQL without indentation (legacy compatibility)
 */
export function formatSql(sql: string): string {
  return formatSqlWithIndentation(sql);
}
