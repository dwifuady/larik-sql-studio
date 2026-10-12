/**
 * PostgreSQL dialect.
 */
import type { ParameterInfo, TableInfo } from '../types';
import { getDisplayDataType } from '../types';
import { quotePostgresIdent } from '../utils/sql/ident';
import { splitPostgresStatements } from '../utils/sql/splitPostgres';
import type { SqlDialect } from './types';

const KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'AND', 'OR', 'NOT', 'IN', 'LIKE', 'ILIKE', 'SIMILAR TO',
  'ORDER BY', 'GROUP BY', 'HAVING', 'JOIN', 'LEFT JOIN', 'RIGHT JOIN',
  'INNER JOIN', 'FULL JOIN', 'OUTER JOIN', 'CROSS JOIN', 'LATERAL', 'ON', 'USING', 'AS',
  'DISTINCT', 'DISTINCT ON', 'LIMIT', 'OFFSET', 'FETCH FIRST', 'ROWS ONLY',
  'INSERT', 'INTO', 'VALUES', 'DEFAULT VALUES', 'ON CONFLICT', 'DO NOTHING', 'DO UPDATE SET',
  'EXCLUDED', 'RETURNING', 'UPDATE', 'SET', 'DELETE', 'TRUNCATE',
  'CREATE', 'ALTER', 'DROP', 'TABLE', 'INDEX', 'VIEW', 'MATERIALIZED VIEW',
  'FUNCTION', 'PROCEDURE', 'TRIGGER', 'SCHEMA', 'SEQUENCE', 'EXTENSION', 'TYPE',
  'IF EXISTS', 'IF NOT EXISTS', 'CASCADE', 'CONCURRENTLY',
  'NULL', 'IS NULL', 'IS NOT NULL', 'IS DISTINCT FROM', 'IS NOT DISTINCT FROM',
  'BETWEEN', 'EXISTS', 'ANY', 'ALL', 'TRUE', 'FALSE',
  'UNION', 'UNION ALL', 'EXCEPT', 'INTERSECT',
  'CASE', 'WHEN', 'THEN', 'ELSE', 'END', 'CAST', 'INTERVAL',
  'BEGIN', 'COMMIT', 'ROLLBACK', 'SAVEPOINT', 'START TRANSACTION',
  'WITH', 'WITH RECURSIVE', 'OVER', 'PARTITION BY', 'WINDOW', 'FILTER',
  'GROUPING SETS', 'ROLLUP', 'CUBE',
  'ASC', 'DESC', 'NULLS FIRST', 'NULLS LAST',
  'CALL', 'DO', 'EXPLAIN', 'EXPLAIN ANALYZE', 'ANALYZE', 'VACUUM',
  'REFRESH MATERIALIZED VIEW', 'GRANT', 'REVOKE', 'SHOW', 'COPY', 'TABLESAMPLE',
];

const FUNCTIONS = [
  // Aggregate functions
  'COUNT', 'SUM', 'AVG', 'MIN', 'MAX', 'STRING_AGG', 'ARRAY_AGG',
  'JSON_AGG', 'JSONB_AGG', 'JSON_OBJECT_AGG', 'JSONB_OBJECT_AGG', 'BOOL_AND', 'BOOL_OR',
  // String functions
  'LENGTH', 'CHAR_LENGTH', 'LOWER', 'UPPER', 'INITCAP', 'SUBSTRING', 'SUBSTR',
  'POSITION', 'STRPOS', 'REPLACE', 'TRIM', 'LTRIM', 'RTRIM', 'BTRIM', 'LPAD', 'RPAD',
  'CONCAT', 'CONCAT_WS', 'SPLIT_PART', 'LEFT', 'RIGHT', 'REVERSE', 'REPEAT', 'FORMAT',
  'REGEXP_REPLACE', 'REGEXP_MATCHES', 'REGEXP_SPLIT_TO_TABLE', 'MD5', 'TO_CHAR',
  // Date/Time functions
  'NOW', 'CURRENT_DATE', 'CURRENT_TIMESTAMP', 'LOCALTIMESTAMP', 'CLOCK_TIMESTAMP',
  'DATE_TRUNC', 'DATE_PART', 'EXTRACT', 'AGE', 'MAKE_DATE', 'MAKE_INTERVAL',
  'TO_DATE', 'TO_TIMESTAMP',
  // Conversion and conditional functions
  'CAST', 'TO_NUMBER', 'COALESCE', 'NULLIF', 'GREATEST', 'LEAST',
  // Math functions
  'ABS', 'CEIL', 'FLOOR', 'ROUND', 'TRUNC', 'POWER', 'SQRT', 'MOD', 'SIGN',
  'LN', 'LOG', 'EXP', 'PI', 'RANDOM',
  // Window functions
  'ROW_NUMBER', 'RANK', 'DENSE_RANK', 'NTILE', 'LAG', 'LEAD',
  'FIRST_VALUE', 'LAST_VALUE', 'NTH_VALUE', 'PERCENT_RANK', 'CUME_DIST',
  // Array functions
  'ARRAY_LENGTH', 'CARDINALITY', 'UNNEST', 'ARRAY_POSITION', 'ARRAY_APPEND', 'ARRAY_REMOVE',
  // JSON functions
  'JSON_BUILD_OBJECT', 'JSONB_BUILD_OBJECT', 'JSONB_EXTRACT_PATH_TEXT',
  'JSONB_ARRAY_ELEMENTS', 'JSONB_EACH', 'JSONB_SET', 'TO_JSONB', 'JSONB_PRETTY', 'JSONB_TYPEOF',
  // Set-returning and system functions
  'GENERATE_SERIES', 'CURRENT_DATABASE', 'CURRENT_SCHEMA', 'VERSION', 'PG_TYPEOF',
  'GEN_RANDOM_UUID', 'PG_SIZE_PRETTY', 'PG_TOTAL_RELATION_SIZE',
  'PG_CANCEL_BACKEND', 'PG_TERMINATE_BACKEND', 'NEXTVAL', 'CURRVAL', 'SETVAL',
  // Full-text search
  'TO_TSVECTOR', 'TO_TSQUERY', 'PLAINTO_TSQUERY',
];

const JOIN_KEYWORDS = ['FROM', 'JOIN', 'INNER JOIN', 'LEFT JOIN', 'RIGHT JOIN', 'FULL JOIN', 'OUTER JOIN', 'CROSS JOIN'];

function quoteQualified(schema: string, name: string): string {
  return `${quotePostgresIdent(schema)}.${quotePostgresIdent(name)}`;
}

function objectReference(schema: string, name: string): string {
  return schema === 'public' ? quotePostgresIdent(name) : quoteQualified(schema, name);
}

function quoteString(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

/** Positional call: `f(${1:NULL}, ${2:NULL})` over the input parameters. */
function positionalCall(target: string, parameters: ParameterInfo[]): string {
  const args = parameters
    .filter(p => p.parameter_mode !== 'OUT')
    .map((_, index) => `\${${index + 1}:NULL}`)
    .join(', ');
  return `${target}(${args})`;
}

export const postgresDialect: SqlDialect = {
  driver: 'postgres',
  monacoLanguage: 'pgsql',
  parserDatabases: ['PostgreSQL'],
  formatterLanguage: 'postgresql',
  identifierQuote: 'double',
  defaultSchema: 'public',
  systemSchemas: ['public', 'pg_catalog', 'information_schema'],
  keywords: KEYWORDS,
  functions: FUNCTIONS,
  tableContextKeywords: [...JOIN_KEYWORDS, 'LATERAL', 'INTO', 'UPDATE', 'TABLE', 'TRUNCATE', 'USING'],
  tableAliasKeywords: JOIN_KEYWORDS,
  routineContextKeywords: ['CALL'],
  features: {
    useDatabase: false,
    localVariables: false,
    batchSeparator: false,
    semanticValidation: false,
    gridEdit: false,
    foreignKeyNavigation: false,
  },

  quoteIdent: quotePostgresIdent,
  quoteQualified,
  objectReference,
  quoteString,
  splitStatements: splitPostgresStatements,

  selectTopQuery: (schema, table, limit) =>
    `SELECT * FROM ${quoteQualified(schema, table)} LIMIT ${limit}`,

  objectDefinitionQuery(kind, schema, name) {
    const qualified = quoteString(quoteQualified(schema, name));
    if (kind === 'view') {
      return `SELECT 'CREATE OR REPLACE VIEW ' || ${qualified} || E' AS\\n' || pg_get_viewdef(${qualified}::regclass, true) AS definition`;
    }
    // Every overload of the routine, separated by blank lines.
    return `SELECT string_agg(pg_get_functiondef(p.oid), E'\\n\\n' ORDER BY p.oid) AS definition `
      + `FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace `
      + `WHERE n.nspname = ${quoteString(schema)} AND p.proname = ${quoteString(name)}`;
  },

  createTableScript(table: TableInfo): string {
    const columns = [...table.columns].sort((a, b) => a.ordinal_position - b.ordinal_position);
    const columnLines = columns.map((col) => {
      const parts = [`    ${quotePostgresIdent(col.name)}`, getDisplayDataType(col)];
      if (col.is_identity) parts.push('GENERATED BY DEFAULT AS IDENTITY');
      if (!col.is_nullable) parts.push('NOT NULL');
      if (col.column_default) parts.push(`DEFAULT ${col.column_default}`);
      return parts.join(' ');
    });
    const pkColumns = columns.filter((c) => c.is_primary_key).map((c) => quotePostgresIdent(c.name));
    if (pkColumns.length > 0) {
      columnLines.push(`    PRIMARY KEY (${pkColumns.join(', ')})`);
    }
    return `CREATE TABLE ${quoteQualified(table.schema_name, table.table_name)} (\n${columnLines.join(',\n')}\n);`;
  },

  procedureCallSnippet: (schema, name, parameters) =>
    positionalCall(objectReference(schema, name), parameters),

  functionCallSnippet: (schema, name, parameters) =>
    positionalCall(objectReference(schema, name), parameters),
};
